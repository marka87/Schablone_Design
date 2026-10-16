//! RS-274X (Extended Gerber) Leser und Schreiber.
//!
//! Leser: Formatspezifikation (FS/MO), Standard-Aperturen (C/R/O/P), Aperturmakros (AM) mit
//! Variablen und Ausdrücken, Linien/Bögen (G01/G02/G03, G74/G75), Regionen (G36/G37),
//! Polarität (LP), Step & Repeat (SR) sowie Apertur-Rotation/-Skalierung (LR/LS).
//! Schreiber: Format 4.6 mm, deduplizierte Flash-Aperturen, Polygone als G36/G37-Regionen.

use super::geometry::{arc_points, bounds, circle_pts, convex_hull, polygon_area, rect_pts, rotate_pts, stadium_pts};
use super::types::{ApertureItem, PadBox, Prim, Pt, StencilZone};
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub enum FlashGeom {
    Circle(f64),
    Rect(f64, f64),
    /// Polygone relativ zum Flash-Punkt
    Polys(Vec<Vec<Pt>>),
}

#[derive(Debug, Clone)]
pub enum GObj {
    Flash { x: f64, y: f64, geom: FlashGeom },
    Seg { x1: f64, y1: f64, x2: f64, y2: f64, w: f64 },
    Region(Vec<Pt>),
    /// Gezeichneter Linienzug (D02 … D01 …), für Konturen und Zonenerkennung
    Path(Vec<Pt>),
}

#[derive(Debug, Default)]
pub struct GerberData {
    pub objects: Vec<GObj>,
}

#[derive(Debug, Clone)]
enum ApDef {
    Circle(f64),
    Rect(f64, f64),
    Obround(f64, f64),
    Polygon { d: f64, n: usize, rot: f64 },
    Macro { name: String, params: Vec<f64> },
}

#[derive(Debug, Clone)]
struct Macro {
    body: Vec<String>,
}

// -------------------------------------------------------------
// Ausdrucksauswertung für Aperturmakros
// -------------------------------------------------------------
struct Expr<'a> {
    s: &'a [u8],
    pos: usize,
    vars: &'a HashMap<usize, f64>,
}

impl<'a> Expr<'a> {
    fn eval(src: &str, vars: &HashMap<usize, f64>) -> f64 {
        let cleaned: String = src.chars().filter(|c| !c.is_whitespace()).collect();
        let mut e = Expr { s: cleaned.as_bytes(), pos: 0, vars };
        let v = e.expr();
        if v.is_finite() { v } else { 0.0 }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }
    fn expr(&mut self) -> f64 {
        let mut v = self.term();
        while let Some(c) = self.peek() {
            match c {
                b'+' => { self.pos += 1; v += self.term(); }
                b'-' => { self.pos += 1; v -= self.term(); }
                _ => break,
            }
        }
        v
    }
    fn term(&mut self) -> f64 {
        let mut v = self.factor();
        while let Some(c) = self.peek() {
            match c {
                b'x' | b'X' => { self.pos += 1; v *= self.factor(); }
                b'/' => { self.pos += 1; let d = self.factor(); v = if d != 0.0 { v / d } else { 0.0 }; }
                _ => break,
            }
        }
        v
    }
    fn factor(&mut self) -> f64 {
        match self.peek() {
            Some(b'-') => { self.pos += 1; -self.factor() }
            Some(b'+') => { self.pos += 1; self.factor() }
            Some(b'(') => {
                self.pos += 1;
                let v = self.expr();
                if self.peek() == Some(b')') {
                    self.pos += 1;
                }
                v
            }
            Some(b'$') => {
                self.pos += 1;
                let start = self.pos;
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
                let idx: usize = std::str::from_utf8(&self.s[start..self.pos]).ok().and_then(|t| t.parse().ok()).unwrap_or(0);
                self.vars.get(&idx).copied().unwrap_or(0.0)
            }
            _ => {
                let start = self.pos;
                while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
                    self.pos += 1;
                }
                if start == self.pos {
                    self.pos += 1; // unbekanntes Zeichen überspringen
                    return 0.0;
                }
                std::str::from_utf8(&self.s[start..self.pos]).ok().and_then(|t| t.parse().ok()).unwrap_or(0.0)
            }
        }
    }
}

/// Wertet ein Aperturmakro aus und liefert belichtete Polygone (relativ zum Flash-Ursprung).
fn eval_macro(m: &Macro, params: &[f64]) -> Vec<Vec<Pt>> {
    let mut vars: HashMap<usize, f64> = params.iter().enumerate().map(|(i, v)| (i + 1, *v)).collect();
    let mut polys = Vec::new();
    for stmt in &m.body {
        let stmt = stmt.trim();
        if stmt.is_empty() || stmt.starts_with('0') && (stmt.len() == 1 || stmt.as_bytes()[1] == b' ' || stmt.as_bytes()[1] == b',') {
            continue;
        }
        if let Some(rest) = stmt.strip_prefix('$') {
            if let Some((lhs, rhs)) = rest.split_once('=') {
                if let Ok(idx) = lhs.trim().parse::<usize>() {
                    let v = Expr::eval(rhs, &vars);
                    vars.insert(idx, v);
                }
            }
            continue;
        }
        let parts: Vec<f64> = stmt.split(',').map(|p| Expr::eval(p, &vars)).collect();
        if parts.is_empty() {
            continue;
        }
        let code = parts[0].round() as i64;
        let a = |i: usize| parts.get(i).copied().unwrap_or(0.0);
        let exposure_on = a(1) != 0.0;
        let poly: Option<Vec<Pt>> = match code {
            1 => {
                let r = a(2) / 2.0;
                let pts = circle_pts(a(3), a(4), r, 32);
                Some(rotate_pts(&pts, 0.0, 0.0, a(5)))
            }
            2 | 20 => {
                let (w, xs, ys, xe, ye, rot) = (a(2), a(3), a(4), a(5), a(6), a(7));
                let (dx, dy) = (xe - xs, ye - ys);
                let len = dx.hypot(dy).max(1e-12);
                let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
                let pts = vec![(xs + nx, ys + ny), (xe + nx, ye + ny), (xe - nx, ye - ny), (xs - nx, ys - ny)];
                Some(rotate_pts(&pts, 0.0, 0.0, rot))
            }
            21 => Some(rotate_pts(&rect_pts(a(4), a(5), a(2), a(3)), 0.0, 0.0, a(6))),
            22 => Some(rotate_pts(&rect_pts(a(4) + a(2) / 2.0, a(5) + a(3) / 2.0, a(2), a(3)), 0.0, 0.0, a(6))),
            4 => {
                let n = a(2).round().max(0.0) as usize;
                let mut pts = Vec::with_capacity(n + 1);
                for i in 0..=n {
                    pts.push((a(3 + 2 * i), a(4 + 2 * i)));
                }
                if pts.len() > 1 && pts.first() == pts.last() {
                    pts.pop();
                }
                let rot = a(5 + 2 * n);
                Some(rotate_pts(&pts, 0.0, 0.0, rot))
            }
            5 => {
                let n = a(2).round().max(3.0) as usize;
                let (cx, cy, r, rot) = (a(3), a(4), a(5) / 2.0, a(6));
                let pts: Vec<Pt> = (0..n)
                    .map(|i| {
                        let ang = (2.0 * std::f64::consts::PI * i as f64 / n as f64) + rot.to_radians();
                        (cx + r * ang.cos(), cy + r * ang.sin())
                    })
                    .collect();
                Some(pts)
            }
            6 => Some(rotate_pts(&circle_pts(a(1), a(2), a(3) / 2.0, 32), 0.0, 0.0, a(9))),
            7 => Some(rotate_pts(&circle_pts(a(1), a(2), a(3) / 2.0, 32), 0.0, 0.0, a(6))),
            _ => None,
        };
        // Primitive 6/7 haben kein Exposure-Feld
        let exposed = matches!(code, 6 | 7) || exposure_on;
        if let Some(p) = poly {
            if exposed && p.len() >= 3 {
                polys.push(p);
            }
        }
    }
    polys
}

// -------------------------------------------------------------
// Parser
// -------------------------------------------------------------
struct State {
    int_digits: u32,
    dec_digits: u32,
    trailing: bool,
    incremental: bool,
    scale: f64, // nach mm
    apertures: HashMap<u32, ApDef>,
    macros: HashMap<String, Macro>,
    current_ap: Option<u32>,
    x: f64,
    y: f64,
    interp: u8, // 1 linear, 2 cw, 3 ccw
    multi_quadrant: Option<bool>,
    d_mode: u8,
    in_region: bool,
    contour: Vec<Pt>,
    path: Vec<Pt>,
    dark: bool,
    rot: f64,
    ap_scale: f64,
    sr: Option<(usize, usize, usize, f64, f64)>, // (start_idx, nx, ny, dx, dy)
}

impl State {
    fn coord(&self, s: &str) -> f64 {
        let neg = s.starts_with('-');
        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        if s.contains('.') {
            return s.parse::<f64>().unwrap_or(0.0) * self.scale;
        }
        let total = (self.int_digits + self.dec_digits) as usize;
        let mut d = digits;
        if self.trailing && d.len() < total {
            d.push_str(&"0".repeat(total - d.len()));
        }
        let v = d.parse::<f64>().unwrap_or(0.0) / 10f64.powi(self.dec_digits as i32);
        (if neg { -v } else { v }) * self.scale
    }

    fn ap_width(&self) -> f64 {
        match self.current_ap.and_then(|c| self.apertures.get(&c)) {
            Some(ApDef::Circle(d)) => *d,
            Some(ApDef::Rect(w, h)) | Some(ApDef::Obround(w, h)) => w.min(*h),
            Some(ApDef::Polygon { d, .. }) => *d,
            _ => 0.0,
        }
    }

    fn flash_geom(&self) -> Option<FlashGeom> {
        let ap = self.apertures.get(&self.current_ap?)?;
        let s = self.ap_scale;
        let geom = match ap {
            ApDef::Circle(d) => FlashGeom::Circle(d * s),
            ApDef::Rect(w, h) => FlashGeom::Rect(w * s, h * s),
            ApDef::Obround(w, h) => FlashGeom::Polys(vec![stadium_pts(0.0, 0.0, w * s, h * s, 8)]),
            ApDef::Polygon { d, n, rot } => {
                let r = d * s / 2.0;
                let pts = (0..*n.max(&3))
                    .map(|i| {
                        let a = 2.0 * std::f64::consts::PI * i as f64 / *n.max(&3) as f64 + rot.to_radians();
                        (r * a.cos(), r * a.sin())
                    })
                    .collect();
                FlashGeom::Polys(vec![pts])
            }
            ApDef::Macro { name, params } => {
                let m = self.macros.get(name)?;
                let polys: Vec<Vec<Pt>> = eval_macro(m, params)
                    .into_iter()
                    .map(|p| p.into_iter().map(|(x, y)| (x * self.scale * s, y * self.scale * s)).collect())
                    .collect();
                if polys.is_empty() {
                    return None;
                }
                FlashGeom::Polys(polys)
            }
        };
        if self.rot.abs() > 1e-9 {
            return Some(match geom {
                FlashGeom::Circle(d) => FlashGeom::Circle(d),
                FlashGeom::Rect(w, h) => {
                    let q = (self.rot / 90.0).round();
                    if (self.rot - q * 90.0).abs() < 1e-6 {
                        if (q as i64).rem_euclid(2) == 1 { FlashGeom::Rect(h, w) } else { FlashGeom::Rect(w, h) }
                    } else {
                        FlashGeom::Polys(vec![rotate_pts(&rect_pts(0.0, 0.0, w, h), 0.0, 0.0, self.rot)])
                    }
                }
                FlashGeom::Polys(ps) => FlashGeom::Polys(ps.iter().map(|p| rotate_pts(p, 0.0, 0.0, self.rot)).collect()),
            });
        }
        Some(geom)
    }
}

fn parse_ad(body: &str, st: &mut State) {
    // ADD<code><name>[,<p1>X<p2>...]
    let rest = &body[3..];
    let num_len = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    let Ok(code) = rest[..num_len].parse::<u32>() else { return };
    let rest = &rest[num_len..];
    let (name, params) = match rest.split_once(',') {
        Some((n, p)) => (n, p),
        None => (rest, ""),
    };
    let vals: Vec<f64> = params.split('X').filter(|s| !s.is_empty()).map(|s| s.trim().parse::<f64>().unwrap_or(0.0)).collect();
    let v = |i: usize| vals.get(i).copied().unwrap_or(0.0) * st.scale;
    let def = match name {
        "C" => ApDef::Circle(v(0)),
        "R" => ApDef::Rect(v(0), v(1)),
        "O" => ApDef::Obround(v(0), v(1)),
        "P" => ApDef::Polygon { d: v(0), n: vals.get(1).copied().unwrap_or(4.0).round() as usize, rot: vals.get(2).copied().unwrap_or(0.0) },
        other => ApDef::Macro { name: other.to_string(), params: vals.clone() },
    };
    st.apertures.insert(code, def);
}

fn close_sr(st: &mut State, objects: &mut Vec<GObj>) {
    if let Some((start, nx, ny, dx, dy)) = st.sr.take() {
        let block: Vec<GObj> = objects[start..].to_vec();
        for i in 0..nx {
            for j in 0..ny {
                if i == 0 && j == 0 {
                    continue;
                }
                let (ox, oy) = (i as f64 * dx, j as f64 * dy);
                for o in &block {
                    objects.push(match o {
                        GObj::Flash { x, y, geom } => GObj::Flash { x: x + ox, y: y + oy, geom: geom.clone() },
                        GObj::Seg { x1, y1, x2, y2, w } => GObj::Seg { x1: x1 + ox, y1: y1 + oy, x2: x2 + ox, y2: y2 + oy, w: *w },
                        GObj::Region(p) => GObj::Region(p.iter().map(|&(x, y)| (x + ox, y + oy)).collect()),
                        GObj::Path(p) => GObj::Path(p.iter().map(|&(x, y)| (x + ox, y + oy)).collect()),
                    });
                }
            }
        }
    }
}

fn flush_path(st: &mut State, objects: &mut Vec<GObj>) {
    if st.path.len() >= 2 && st.dark {
        objects.push(GObj::Path(std::mem::take(&mut st.path)));
    } else {
        st.path.clear();
    }
}

fn flush_contour(st: &mut State, objects: &mut Vec<GObj>) {
    let mut c = std::mem::take(&mut st.contour);
    if c.len() > 1 && c.first() == c.last() {
        c.pop();
    }
    if c.len() >= 3 && st.dark && polygon_area(&c) > 1e-9 {
        objects.push(GObj::Region(c));
    }
}

/// Ermittelt das Bogenzentrum (Multi- oder Single-Quadrant).
fn arc_center(st: &State, start: Pt, end: Pt, i: f64, j: f64, cw: bool) -> Pt {
    let multi = st.multi_quadrant.unwrap_or(true);
    if multi {
        let c = (start.0 + i, start.1 + j);
        let err = ((start.0 - c.0).hypot(start.1 - c.1) - (end.0 - c.0).hypot(end.1 - c.1)).abs();
        if st.multi_quadrant.is_some() || err < 1e-3_f64.max(0.01 * i.hypot(j)) {
            return c;
        }
    }
    // Single-Quadrant: Vorzeichen so wählen, dass Radius konsistent und Bogen ≤ 90°
    let mut best = (start.0 + i.abs(), start.1 + j.abs());
    let mut best_score = f64::INFINITY;
    for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        let c = (start.0 + sx * i.abs(), start.1 + sy * j.abs());
        let r0 = (start.0 - c.0).hypot(start.1 - c.1);
        let r1 = (end.0 - c.0).hypot(end.1 - c.1);
        let a0 = (start.1 - c.1).atan2(start.0 - c.0);
        let a1 = (end.1 - c.1).atan2(end.0 - c.0);
        let mut sweep = if cw { a0 - a1 } else { a1 - a0 };
        while sweep < 0.0 {
            sweep += std::f64::consts::TAU;
        }
        let score = (r0 - r1).abs() + if sweep > std::f64::consts::FRAC_PI_2 + 1e-6 { 1e3 } else { 0.0 };
        if score < best_score {
            best_score = score;
            best = c;
        }
    }
    best
}

pub fn parse_gerber(content: &str) -> GerberData {
    let mut st = State {
        int_digits: 2,
        dec_digits: 4,
        trailing: false,
        incremental: false,
        scale: 25.4,
        apertures: HashMap::new(),
        macros: HashMap::new(),
        current_ap: None,
        x: 0.0,
        y: 0.0,
        interp: 1,
        multi_quadrant: None,
        d_mode: 1,
        in_region: false,
        contour: Vec::new(),
        path: Vec::new(),
        dark: true,
        rot: 0.0,
        ap_scale: 1.0,
        sr: None,
    };
    let mut objects: Vec<GObj> = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        if c == b'%' {
            let end = content[i + 1..].find('%').map(|p| i + 1 + p).unwrap_or(bytes.len());
            let block: String = content[i + 1..end].chars().filter(|c| *c != '\r' && *c != '\n').collect();
            i = end + 1;
            handle_extended(&block, &mut st, &mut objects);
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // Wort endet an '*'; tolerant gegenüber fehlendem '*' vor einem '%'-Block (Gerbonara schreibt "G75" ohne '*')
        let end = content[i..].find(['*', '%']).map(|p| i + p).unwrap_or(bytes.len());
        let word: String = content[i..end].chars().filter(|c| !c.is_whitespace()).collect();
        i = if bytes.get(end) == Some(&b'*') { end + 1 } else { end };
        handle_word(&word, &mut st, &mut objects);
    }
    flush_path(&mut st, &mut objects);
    if st.in_region {
        flush_contour(&mut st, &mut objects);
    }
    close_sr(&mut st, &mut objects);
    GerberData { objects }
}

fn handle_extended(block: &str, st: &mut State, objects: &mut Vec<GObj>) {
    let cmds: Vec<&str> = block.split('*').map(str::trim).filter(|s| !s.is_empty()).collect();
    let Some(first) = cmds.first() else { return };
    if first.starts_with("AM") {
        let name = first[2..].to_string();
        st.macros.insert(name, Macro { body: cmds[1..].iter().map(|s| s.to_string()).collect() });
        return;
    }
    for cmd in cmds {
        if let Some(rest) = cmd.strip_prefix("FS") {
            st.trailing = rest.contains('T') && !rest.contains('L');
            st.incremental = rest.contains('I') && !rest.contains("A");
            if let Some(p) = rest.find('X') {
                let d: Vec<u32> = rest[p + 1..].chars().take(2).filter_map(|c| c.to_digit(10)).collect();
                if d.len() == 2 {
                    st.int_digits = d[0];
                    st.dec_digits = d[1];
                }
            }
        } else if cmd.starts_with("MOMM") {
            st.scale = 1.0;
        } else if cmd.starts_with("MOIN") {
            st.scale = 25.4;
        } else if cmd.starts_with("ADD") {
            parse_ad(cmd, st);
        } else if cmd.starts_with("LPD") {
            flush_path(st, objects);
            st.dark = true;
        } else if cmd.starts_with("LPC") {
            flush_path(st, objects);
            st.dark = false;
        } else if let Some(r) = cmd.strip_prefix("LR") {
            st.rot = r.parse().unwrap_or(0.0);
        } else if let Some(s) = cmd.strip_prefix("LS") {
            st.ap_scale = s.parse().unwrap_or(1.0);
        } else if let Some(rest) = cmd.strip_prefix("SR") {
            flush_path(st, objects);
            close_sr(st, objects);
            if !rest.is_empty() {
                let num = |key: char| -> f64 {
                    rest.find(key)
                        .map(|p| rest[p + 1..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-').collect::<String>())
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0)
                };
                let (nx, ny) = (num('X').max(1.0) as usize, num('Y').max(1.0) as usize);
                if nx > 1 || ny > 1 {
                    st.sr = Some((objects.len(), nx, ny, num('I') * st.scale, num('J') * st.scale));
                }
            }
        }
    }
}

fn handle_word(word: &str, st: &mut State, objects: &mut Vec<GObj>) {
    if word.is_empty() || word.starts_with("G04") || word.starts_with("G4 ") {
        return;
    }
    let b = word.as_bytes();
    let mut k = 0;
    let (mut nx, mut ny, mut ni, mut nj): (Option<f64>, Option<f64>, f64, f64) = (None, None, 0.0, 0.0);
    let mut dcode: Option<u32> = None;
    while k < b.len() {
        let letter = b[k] as char;
        k += 1;
        let start = k;
        while k < b.len() && (b[k].is_ascii_digit() || b[k] == b'-' || b[k] == b'+' || b[k] == b'.') {
            k += 1;
        }
        let val = &word[start..k];
        match letter {
            'G' => match val.parse::<u32>().unwrap_or(999) {
                1 => st.interp = 1,
                2 => st.interp = 2,
                3 => st.interp = 3,
                36 => {
                    flush_path(st, objects);
                    st.in_region = true;
                    st.contour.clear();
                }
                37 => {
                    flush_contour(st, objects);
                    st.in_region = false;
                }
                74 => st.multi_quadrant = Some(false),
                75 => st.multi_quadrant = Some(true),
                70 => st.scale = 25.4,
                71 => st.scale = 1.0,
                90 => st.incremental = false,
                91 => st.incremental = true,
                4 => return,
                _ => {}
            },
            'X' => nx = Some(st.coord(val)),
            'Y' => ny = Some(st.coord(val)),
            'I' => ni = st.coord(val),
            'J' => nj = st.coord(val),
            'D' => dcode = val.parse().ok(),
            'M' => {
                if val == "02" || val == "2" || val == "00" || val == "0" {
                    flush_path(st, objects);
                }
                return;
            }
            'N' => {}
            _ => {}
        }
    }

    if let Some(d) = dcode {
        if d >= 10 {
            flush_path(st, objects);
            st.current_ap = Some(d);
            return;
        }
        st.d_mode = d as u8;
    } else if nx.is_none() && ny.is_none() {
        return;
    }

    let (tx, ty) = if st.incremental {
        (st.x + nx.unwrap_or(0.0), st.y + ny.unwrap_or(0.0))
    } else {
        (nx.unwrap_or(st.x), ny.unwrap_or(st.y))
    };
    let start = (st.x, st.y);
    let end = (tx, ty);

    match st.d_mode {
        1 => {
            let pts: Vec<Pt> = if st.interp == 1 {
                vec![end]
            } else {
                let cw = st.interp == 2;
                let center = arc_center(st, start, end, ni, nj, cw);
                arc_points(start, end, center, cw)
            };
            if st.in_region {
                if st.contour.is_empty() {
                    st.contour.push(start);
                }
                st.contour.extend(pts);
            } else if st.dark {
                let w = st.ap_width() * st.ap_scale;
                let mut prev = start;
                for p in &pts {
                    objects.push(GObj::Seg { x1: prev.0, y1: prev.1, x2: p.0, y2: p.1, w });
                    prev = *p;
                }
                if st.path.is_empty() {
                    st.path.push(start);
                }
                st.path.extend(pts);
            }
        }
        2 => {
            if st.in_region {
                flush_contour(st, objects);
            } else {
                flush_path(st, objects);
            }
        }
        3 => {
            flush_path(st, objects);
            if st.dark && !st.in_region {
                if let Some(geom) = st.flash_geom() {
                    objects.push(GObj::Flash { x: tx, y: ty, geom });
                }
            }
        }
        _ => {}
    }
    st.x = tx;
    st.y = ty;
}

pub fn parse_gerber_file(path: &std::path::Path) -> Result<GerberData, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(parse_gerber(&String::from_utf8_lossy(&bytes)))
}

impl GerberData {
    fn obj_bounds(o: &GObj) -> Option<(f64, f64, f64, f64)> {
        match o {
            GObj::Flash { x, y, geom } => Some(match geom {
                FlashGeom::Circle(d) => (x - d / 2.0, y - d / 2.0, x + d / 2.0, y + d / 2.0),
                FlashGeom::Rect(w, h) => (x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0),
                FlashGeom::Polys(ps) => {
                    let all: Vec<Pt> = ps.iter().flatten().copied().collect();
                    let b = bounds(&all);
                    (x + b.0, y + b.1, x + b.2, y + b.3)
                }
            }),
            GObj::Seg { x1, y1, x2, y2, w } => Some((x1.min(*x2) - w / 2.0, y1.min(*y2) - w / 2.0, x1.max(*x2) + w / 2.0, y1.max(*y2) + w / 2.0)),
            GObj::Region(p) => Some(bounds(p)),
            GObj::Path(_) => None,
        }
    }

    pub fn bounding_box(&self) -> Option<[f64; 4]> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for o in &self.objects {
            if let Some(ob) = Self::obj_bounds(o) {
                b = [b[0].min(ob.0), b[1].min(ob.1), b[2].max(ob.2), b[3].max(ob.3)];
            }
        }
        if b[0].is_finite() { Some(b) } else { None }
    }

    pub fn to_prims(&self, max: usize) -> Vec<Prim> {
        let mut out = Vec::new();
        for o in &self.objects {
            if out.len() >= max {
                break;
            }
            match o {
                GObj::Flash { x, y, geom } => match geom {
                    FlashGeom::Circle(d) => out.push(Prim::circle(*x, *y, d / 2.0)),
                    FlashGeom::Rect(w, h) => out.push(Prim::rect(*x, *y, *w, *h)),
                    FlashGeom::Polys(ps) => {
                        for p in ps {
                            let abs: Vec<Pt> = p.iter().map(|&(px, py)| (x + px, y + py)).collect();
                            out.push(Prim::poly(&abs));
                        }
                    }
                },
                GObj::Seg { x1, y1, x2, y2, w } => out.push(Prim::line(*x1, *y1, *x2, *y2, *w)),
                GObj::Region(p) => out.push(Prim::poly(p)),
                GObj::Path(_) => {}
            }
        }
        out
    }

    /// Pasten-Aperturen aus Flashes und Regionen (Linien werden wie im Original ignoriert).
    pub fn paste_apertures(&self) -> Vec<ApertureItem> {
        let mut out = Vec::new();
        for o in &self.objects {
            let id = out.len();
            let ap = match o {
                GObj::Flash { x, y, geom } => match geom {
                    FlashGeom::Circle(d) => Some(ApertureItem::new(id, "circle", *x, *y, *d, *d)),
                    FlashGeom::Rect(w, h) => {
                        let mut a = ApertureItem::new(id, "rectangle", *x, *y, *w, *h);
                        a.points = rect_pts(*x, *y, *w, *h);
                        Some(a)
                    }
                    FlashGeom::Polys(ps) => {
                        let all: Vec<Pt> = ps.iter().flatten().map(|&(px, py)| (x + px, y + py)).collect();
                        let poly = if ps.len() == 1 { ps[0].iter().map(|&(px, py)| (x + px, y + py)).collect() } else { convex_hull(&all) };
                        ApertureItem::from_polygon(id, &poly)
                    }
                },
                GObj::Region(p) => ApertureItem::from_polygon(id, p),
                _ => None,
            };
            if let Some(a) = ap.filter(|a| a.area > 1e-5) {
                out.push(a);
            }
        }
        out
    }

    pub fn pad_boxes(&self) -> Vec<PadBox> {
        self.objects
            .iter()
            .filter(|o| matches!(o, GObj::Flash { .. } | GObj::Region(_)))
            .filter_map(Self::obj_bounds)
            .map(|b| PadBox::new(b.0, b.1, b.2, b.3))
            .collect()
    }

    pub fn profile_segments(&self) -> Vec<[Pt; 2]> {
        let mut segs = Vec::new();
        for o in &self.objects {
            match o {
                GObj::Seg { x1, y1, x2, y2, .. } => segs.push([(r2(*x1), r2(*y1)), (r2(*x2), r2(*y2))]),
                GObj::Region(p) => {
                    for i in 0..p.len() {
                        let (a, b) = (p[i], p[(i + 1) % p.len()]);
                        segs.push([(r2(a.0), r2(a.1)), (r2(b.0), r2(b.1))]);
                    }
                }
                _ => {}
            }
        }
        segs
    }

    pub fn fiducials(&self) -> Vec<(f64, f64, f64)> {
        self.objects
            .iter()
            .filter(|o| matches!(o, GObj::Flash { .. } | GObj::Region(_)))
            .filter_map(Self::obj_bounds)
            .map(|b| ((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0, (b.2 - b.0) / 2.0))
            .collect()
    }

    /// Geschlossene Konturen (Regionen und gezeichnete Linienzüge) für die Stufenzonen-Erkennung.
    pub fn contours(&self) -> Vec<Vec<Pt>> {
        self.objects
            .iter()
            .filter_map(|o| match o {
                GObj::Region(p) | GObj::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect()
    }
}

fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// -------------------------------------------------------------
// Schreiber
// -------------------------------------------------------------
fn c46(v: f64) -> i64 {
    (v * 1_000_000.0).round() as i64
}

fn header(title: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "G04 SMT StencilCraft Desktop - {title}*");
    out.push_str("%TF.GenerationSoftware,SMT StencilCraft,Desktop,2.6*%\n");
    out.push_str("%FSLAX46Y46*%\n%MOMM*%\n%LPD*%\n");
    out
}

fn write_region(out: &mut String, pts: &[Pt]) {
    if pts.len() < 3 {
        return;
    }
    out.push_str("G36*\n");
    for (i, p) in pts.iter().enumerate() {
        let _ = writeln!(out, "X{}Y{}D0{}*", c46(p.0), c46(p.1), if i == 0 { 2 } else { 1 });
    }
    let _ = writeln!(out, "X{}Y{}D01*", c46(pts[0].0), c46(pts[0].1));
    out.push_str("G37*\n");
}

/// Schreibt die Arbeits-Aperturen (inkl. Modifikationen) als RS-274X Schablonenlage.
pub fn write_stencil(apertures: &[ApertureItem], title: &str) -> String {
    enum Item<'a> {
        Flash(f64, f64, (char, i64, i64)),
        Region(&'a [Pt]),
    }
    let mut items: Vec<Item> = Vec::new();
    let key = |kind: char, w: f64, h: f64| (kind, (w * 10_000.0).round() as i64, (h * 10_000.0).round() as i64);
    for ap in apertures {
        match ap.custom_slices.as_ref().filter(|s| !s.is_empty()) {
            Some(slices) => {
                for s in slices {
                    if s.is_circle() {
                        items.push(Item::Flash(s.cx, s.cy, key('C', s.w, s.w)));
                    } else if let Some(p) = s.polygon_pts() {
                        items.push(Item::Region(p));
                    } else if s.w > 0.0 && s.h > 0.0 {
                        items.push(Item::Flash(s.cx, s.cy, key('R', s.w, s.h)));
                    }
                }
            }
            None => match ap.shape_type.as_str() {
                "circle" => items.push(Item::Flash(ap.cx, ap.cy, key('C', ap.width, ap.width))),
                "polygon" if ap.points.len() >= 3 => items.push(Item::Region(&ap.points)),
                _ => items.push(Item::Flash(ap.cx, ap.cy, key('R', ap.width, ap.height))),
            },
        }
    }

    let mut codes: Vec<(char, i64, i64)> = Vec::new();
    let mut code_of: HashMap<(char, i64, i64), usize> = HashMap::new();
    for it in &items {
        if let Item::Flash(_, _, k) = it {
            code_of.entry(*k).or_insert_with(|| {
                codes.push(*k);
                10 + codes.len() - 1
            });
        }
    }

    let mut out = header(title);
    for (i, (kind, w, h)) in codes.iter().enumerate() {
        let (w, h) = (*w as f64 / 10_000.0, *h as f64 / 10_000.0);
        if *kind == 'C' {
            let _ = writeln!(out, "%ADD{}C,{:.4}*%", 10 + i, w);
        } else {
            let _ = writeln!(out, "%ADD{}R,{:.4}X{:.4}*%", 10 + i, w, h);
        }
    }
    out.push_str("G01*\n");
    let mut current: Option<usize> = None;
    for it in &items {
        match it {
            Item::Flash(x, y, k) => {
                let code = code_of[k];
                if current != Some(code) {
                    let _ = writeln!(out, "D{code}*");
                    current = Some(code);
                }
                let _ = writeln!(out, "X{}Y{}D03*", c46(*x), c46(*y));
            }
            Item::Region(p) => write_region(&mut out, p),
        }
    }
    out.push_str("M02*\n");
    out
}

/// Stufenzonen als gefüllte Rechteck-Regionen (Fräs-/Ätz-Layer für den Schablonenhersteller).
pub fn write_zones(zones: &[StencilZone], base_thickness: f64) -> String {
    let mut out = header("Step Stencil Zones");
    let _ = writeln!(out, "G04 Basisdicke {:.3} mm*", base_thickness);
    out.push_str("G01*\n");
    for z in zones {
        let _ = writeln!(out, "G04 {} : {:.3} mm*", z.name.replace('*', " "), z.thickness);
        let (x0, y0, x1, y1) = (z.x0.min(z.x1), z.y0.min(z.y1), z.x0.max(z.x1), z.y0.max(z.y1));
        write_region(&mut out, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)]);
    }
    out.push_str("M02*\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flashes_regions_and_macros() {
        let g = "%FSLAX46Y46*%%MOMM*%\n%AMRR*21,1,$1,$2,0,0,0*1,1,$3,0,0*%\n%ADD10R,1.0X0.5*%\n%ADD11C,0.4*%\n%ADD12RR,2X1X0.2*%\nD10*X1000000Y2000000D03*\nD11*X3000000Y2000000D03*\nD12*X5000000Y5000000D03*\nG36*X0Y0D02*X1000000Y0D01*X1000000Y1000000D01*X0Y1000000D01*X0Y0D01*G37*M02*";
        let d = parse_gerber(g);
        let aps = d.paste_apertures();
        assert_eq!(aps.len(), 4);
        assert_eq!(aps[0].shape_type, "rectangle");
        assert!((aps[0].width - 1.0).abs() < 1e-9 && (aps[0].cx - 1.0).abs() < 1e-9);
        assert_eq!(aps[1].shape_type, "circle");
        assert_eq!(aps[2].shape_type, "polygon");
        assert!((aps[2].width - 2.0).abs() < 1e-6);
        assert!((aps[3].area - 1.0).abs() < 1e-9);
    }

    #[test]
    fn inch_leading_zero_format_and_step_repeat() {
        let g = "%FSLAX24Y24*%%MOIN*%%ADD10C,0.010*%%SRX2Y1I0.5J0*%D10*X10000Y10000D03*%SR*%M02*";
        let d = parse_gerber(g);
        let aps = d.paste_apertures();
        assert_eq!(aps.len(), 2);
        assert!((aps[0].cx - 25.4).abs() < 1e-9);
        assert!((aps[1].cx - 38.1).abs() < 1e-9);
    }

    #[test]
    fn tolerates_missing_word_terminator() {
        let g = "%MOMM*%
%FSLAX45Y45*%
G75
%LPD*%
%ADD10C,0.5*%
G36*
G01**
X0Y0D02*
X100000Y0D01*
X100000Y100000D01*
X0Y0D01*
G37*
D10*
X200000Y200000D03*
M02*";
        let aps = parse_gerber(g).paste_apertures();
        assert_eq!(aps.len(), 2);
        assert_eq!(aps[1].shape_type, "circle");
    }

    #[test]
    fn writer_roundtrip() {
        let mut a = ApertureItem::new(0, "rectangle", 1.0, 2.0, 0.5, 0.3);
        a.points = rect_pts(1.0, 2.0, 0.5, 0.3);
        let b = ApertureItem::new(1, "circle", 3.0, 2.0, 0.4, 0.4);
        let text = write_stencil(&[a, b], "Test");
        let back = parse_gerber(&text).paste_apertures();
        assert_eq!(back.len(), 2);
        assert!((back[0].width - 0.5).abs() < 1e-6 && (back[1].width - 0.4).abs() < 1e-6);
    }
}
