//! ODB++ Leser: Archive (.zip, .tgz/.tar.gz), entpackte Job-Ordner und einzelne Step-Ordner.
//!
//! Liest matrix/matrix, steps/<step>/eda/data (Packages, Pins), layers/*/components (Bauteile mit
//! Toeprints), layers/*/features (Pads, Linien, Bögen, Flächen) sowie steps/<step>/profile.

use super::geometry::{arc_points, bounds, circle_pts, rect_pts, rotate_pts, stadium_pts};
use super::types::{r3, Prim, Pt};
use flate2::read::GzDecoder;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub const MAX_LAYER_PRIMS: usize = 150_000;

/// In-Memory-Abbild der relevanten ODB++ Textdateien (Pfade relativ zum Job, klein geschrieben).
pub struct OdbFiles {
    pub files: HashMap<String, String>,
    pub job_name: String,
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_lowercase()
}

fn is_relevant(p: &str) -> bool {
    let p = p.trim_end_matches(".z");
    p.ends_with("matrix/matrix") || p.ends_with("/eda/data") || p.ends_with("/features") || p.ends_with("/components") || p.ends_with("/profile")
}

fn insert_file(files: &mut HashMap<String, String>, name: &str, data: Vec<u8>) {
    let key = norm(name);
    if !is_relevant(&key) {
        return;
    }
    if let Some(stripped) = key.strip_suffix(".z") {
        if let Some(raw) = uncompress_z(&data) {
            files.insert(stripped.to_string(), String::from_utf8_lossy(&raw).into_owned());
        }
    } else {
        files.insert(key, String::from_utf8_lossy(&data).into_owned());
    }
}

fn walk(dir: &Path, base: &Path, files: &mut HashMap<String, String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, files);
        } else if let Ok(rel) = p.strip_prefix(base) {
            let rel_s = rel.to_string_lossy();
            if is_relevant(&norm(&rel_s)) {
                if let Ok(data) = std::fs::read(&p) {
                    insert_file(files, &rel_s, data);
                }
            }
        }
    }
}

pub fn is_archive(path: &Path) -> bool {
    let s = path.to_string_lossy().to_lowercase();
    s.ends_with(".zip") || s.ends_with(".tgz") || s.ends_with(".tar.gz") || s.ends_with(".tar")
}

/// Alle Dateien eines Archivs (für Gerber-ZIPs) als (Name, Bytes).
pub fn read_archive_entries(path: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let s = path.to_string_lossy().to_lowercase();
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Vec::new();
    if s.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| format!("Ungültiges ZIP-Archiv: {e}"))?;
        for i in 0..zip.len() {
            let Ok(mut f) = zip.by_index(i) else { continue };
            if f.is_dir() {
                continue;
            }
            let name = f.name().to_string();
            let mut buf = Vec::new();
            if f.read_to_end(&mut buf).is_ok() {
                out.push((name, buf));
            }
        }
    } else {
        let reader: Box<dyn Read> = if s.ends_with(".tar") { Box::new(file) } else { Box::new(GzDecoder::new(file)) };
        let mut tar = tar::Archive::new(reader);
        let entries = tar.entries().map_err(|e| format!("Ungültiges TAR-Archiv: {e}"))?;
        for entry in entries.flatten() {
            let mut entry = entry;
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let name = entry.path().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let mut buf = Vec::new();
            if entry.read_to_end(&mut buf).is_ok() {
                out.push((name, buf));
            }
        }
    }
    Ok(out)
}

impl OdbFiles {
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut raw = HashMap::new();
        if path.is_dir() {
            walk(path, path, &mut raw);
        } else if is_archive(path) {
            for (name, data) in read_archive_entries(path)? {
                insert_file(&mut raw, &name, data);
            }
        } else {
            return Err(format!("Kein ODB++ Archiv oder Ordner: {}", path.display()));
        }
        let job_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "odb_job".into())
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".zip")
            .to_string();

        // Pfade auf Job-Wurzel normalisieren
        let mut files = HashMap::new();
        let root = raw.keys().find_map(|k| k.strip_suffix("matrix/matrix").map(String::from)).or_else(|| {
            raw.keys().find_map(|k| k.find("steps/").map(|p| k[..p].to_string()))
        });
        match root {
            Some(prefix) => {
                for (k, v) in raw {
                    if let Some(rel) = k.strip_prefix(&prefix) {
                        files.insert(rel.to_string(), v);
                    }
                }
            }
            None => {
                // Einzelner Step-Ordner: .../layers/<name>/features → steps/<job>/layers/<name>/features
                let step = norm(&job_name);
                for (k, v) in raw {
                    if let Some(p) = k.find("layers/") {
                        let prefix_is_boundary = p == 0 || k.as_bytes()[p - 1] == b'/';
                        if prefix_is_boundary {
                            files.insert(format!("steps/{step}/{}", &k[p..]), v);
                            continue;
                        }
                    }
                    if k.ends_with("eda/data") {
                        files.insert(format!("steps/{step}/eda/data"), v);
                    } else if k == "profile" || k.ends_with("/profile") {
                        files.insert(format!("steps/{step}/profile"), v);
                    }
                }
            }
        }
        if files.is_empty() {
            return Err("Keine gültigen ODB++ Daten gefunden (matrix/steps/layers fehlen).".into());
        }
        Ok(OdbFiles { files, job_name })
    }

    pub fn steps(&self) -> Vec<String> {
        let mut s: Vec<String> = self
            .files
            .keys()
            .filter_map(|k| k.strip_prefix("steps/").and_then(|r| r.split('/').next()).map(String::from))
            .collect();
        s.sort();
        s.dedup();
        s
    }

    /// Board-Step wählen (Nutzen-/Panel-Steps ausblenden, Steps mit EDA-Daten bevorzugen).
    pub fn board_step(&self) -> Option<String> {
        let steps = self.steps();
        let is_panel = |s: &str| ["panel", "nutz", "template", "array"].iter().any(|x| s.contains(x));
        let with_eda = |s: &str| self.files.contains_key(&format!("steps/{s}/eda/data"));
        steps
            .iter()
            .find(|s| !is_panel(s) && with_eda(s))
            .or_else(|| steps.iter().find(|s| !is_panel(s)))
            .or_else(|| steps.first())
            .cloned()
    }

    pub fn get(&self, rel: &str) -> Option<&String> {
        self.files.get(rel)
    }
}

// -------------------------------------------------------------
// Matrix
// -------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct MatrixLayer {
    pub name: String,
    pub raw_name: String,
    pub ltype: String,
    pub row: i64,
    /// Bohrlagen: Start-/Endlage (klein geschrieben), leer wenn nicht angegeben
    pub start: String,
    pub end: String,
}

pub fn parse_matrix(content: &str) -> Vec<MatrixLayer> {
    let mut out = Vec::new();
    let mut in_layer = false;
    let mut cur: HashMap<String, String> = HashMap::new();
    for line in content.lines() {
        let l = line.trim();
        if l.starts_with("LAYER") && l.contains('{') {
            in_layer = true;
            cur.clear();
        } else if l.starts_with('}') {
            if in_layer {
                if let Some(name) = cur.get("NAME") {
                    out.push(MatrixLayer {
                        name: name.to_lowercase(),
                        raw_name: name.clone(),
                        ltype: cur.get("TYPE").cloned().unwrap_or_default().to_uppercase(),
                        row: cur.get("ROW").and_then(|r| r.parse().ok()).unwrap_or(out.len() as i64),
                        start: cur.get("START_NAME").map(|s| s.to_lowercase()).unwrap_or_default(),
                        end: cur.get("END_NAME").map(|s| s.to_lowercase()).unwrap_or_default(),
                    });
                }
            }
            in_layer = false;
        } else if in_layer {
            if let Some((k, v)) = l.split_once('=') {
                cur.insert(k.trim().to_uppercase(), v.trim().to_string());
            }
        }
    }
    out.sort_by_key(|l| l.row);
    out
}

// -------------------------------------------------------------
// Symbole
// -------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum SymShape {
    Circle,
    Rect { r: f64 },
    Oval,
    Diamond,
    Octagon { c: f64 },
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub shape: SymShape,
    pub w: f64,
    pub h: f64,
}

fn nums(s: &str) -> Vec<f64> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.parse().ok())
        .collect()
}

/// Symbol-Definitionen (`r50`, `s40`, `rect60x20xr3`, `oval60x20`, `di..`, `oct..`, `donut_r..` …).
/// Maße in mil (UNITS=INCH) bzw. µm (UNITS=MM).
pub fn parse_symbol(def: &str, is_inch: bool) -> Symbol {
    let unit = if is_inch { 0.0254 } else { 0.001 };
    let s = def.trim().to_lowercase();
    let n: Vec<f64> = nums(&s).into_iter().map(|v| v * unit).collect();
    let a = |i: usize| n.get(i).copied().unwrap_or(0.5 / unit * unit);
    let sym = |shape: SymShape, w: f64, h: f64| Symbol { shape, w: w.max(0.005), h: h.max(0.005) };
    if s.starts_with("rect") {
        let r = if s.contains("xr") { n.get(2).copied().unwrap_or(0.0) } else { 0.0 };
        sym(SymShape::Rect { r }, a(0), a(1))
    } else if s.starts_with("oval") {
        sym(SymShape::Oval, a(0), a(1))
    } else if s.starts_with("donut_s") || s.starts_with("ths") || s.starts_with("s_t") {
        sym(SymShape::Rect { r: 0.0 }, a(0), a(0))
    } else if s.starts_with("donut_r") || s.starts_with("thr") || s.starts_with("donut") {
        sym(SymShape::Circle, a(0), a(0))
    } else if s.starts_with("di") {
        sym(SymShape::Diamond, a(0), a(1))
    } else if s.starts_with("oct") {
        sym(SymShape::Octagon { c: n.get(2).copied().unwrap_or(a(0).min(a(1)) * 0.29) }, a(0), a(1))
    } else if s.starts_with("el") {
        sym(SymShape::Oval, a(0), a(1))
    } else if s.starts_with('s') && s[1..].chars().next().map_or(false, |c| c.is_ascii_digit()) {
        sym(SymShape::Rect { r: 0.0 }, a(0), a(0))
    } else if s.starts_with('r') && s[1..].chars().next().map_or(false, |c| c.is_ascii_digit()) {
        sym(SymShape::Circle, a(0), a(0))
    } else if n.len() >= 2 {
        sym(SymShape::Rect { r: 0.0 }, n[0], n[1])
    } else if n.len() == 1 {
        sym(SymShape::Circle, n[0], n[0])
    } else {
        sym(SymShape::Rect { r: 0.0 }, 0.5, 0.5)
    }
}

/// Umriss eines Symbols am Ursprung (unrotiert).
pub fn symbol_outline(sym: &Symbol) -> Vec<Pt> {
    let (w, h) = (sym.w, sym.h);
    match &sym.shape {
        SymShape::Circle => circle_pts(0.0, 0.0, w / 2.0, 32),
        SymShape::Rect { r } if *r > 0.005 => {
            let r = r.min(w / 2.0).min(h / 2.0);
            let mut pts = Vec::with_capacity(20);
            for (cx, cy, a0) in [(w / 2.0 - r, h / 2.0 - r, 0.0f64), (-w / 2.0 + r, h / 2.0 - r, 90.0), (-w / 2.0 + r, -h / 2.0 + r, 180.0), (w / 2.0 - r, -h / 2.0 + r, 270.0)] {
                for k in 0..=4 {
                    let a = (a0 + 22.5 * k as f64).to_radians();
                    pts.push((cx + r * a.cos(), cy + r * a.sin()));
                }
            }
            pts
        }
        SymShape::Rect { .. } => rect_pts(0.0, 0.0, w, h),
        SymShape::Oval => stadium_pts(0.0, 0.0, w, h, 8),
        SymShape::Diamond => vec![(w / 2.0, 0.0), (0.0, h / 2.0), (-w / 2.0, 0.0), (0.0, -h / 2.0)],
        SymShape::Octagon { c } => {
            let c = c.min(w / 2.0).min(h / 2.0);
            vec![
                (w / 2.0, h / 2.0 - c), (w / 2.0 - c, h / 2.0), (-w / 2.0 + c, h / 2.0), (-w / 2.0, h / 2.0 - c),
                (-w / 2.0, -h / 2.0 + c), (-w / 2.0 + c, -h / 2.0), (w / 2.0 - c, -h / 2.0), (w / 2.0, -h / 2.0 + c),
            ]
        }
    }
}

// -------------------------------------------------------------
// Features
// -------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum PadGeom {
    Circle { d: f64 },
    Rect { w: f64, h: f64 },
    Poly { pts: Vec<Pt> },
}

#[derive(Debug, Clone)]
pub struct OdbPad {
    pub x: f64,
    pub y: f64,
    pub geom: PadGeom,
}

#[derive(Debug, Default)]
pub struct Features {
    pub prims: Vec<Prim>,
    pub pads: Vec<OdbPad>,
    pub surfaces: Vec<Vec<Pt>>,
    pub bbox: Option<[f64; 4]>,
}

fn strip_attrs(line: &str) -> &str {
    line.split(';').next().unwrap_or(line).trim()
}

fn units_line(l: &str) -> Option<bool> {
    let u = l.strip_prefix("UNITS=").or_else(|| l.strip_prefix("U "))?.trim().to_uppercase();
    Some(u.starts_with("INCH"))
}

/// Wendet ODB++ Orientierung an (Rotation im Uhrzeigersinn, optional gespiegelt).
fn place_pad(sym: &Symbol, x: f64, y: f64, rot_cw: f64, mirror: bool) -> PadGeom {
    let rot = rot_cw.rem_euclid(360.0);
    match sym.shape {
        SymShape::Circle => return PadGeom::Circle { d: sym.w },
        SymShape::Rect { r } if r <= 0.005 => {
            let q = (rot / 90.0).round();
            if (rot - q * 90.0).abs() < 0.01 {
                let (w, h) = if (q as i64) % 2 == 1 { (sym.h, sym.w) } else { (sym.w, sym.h) };
                return PadGeom::Rect { w, h };
            }
        }
        _ => {}
    }
    let mut pts = symbol_outline(sym);
    if mirror {
        pts = pts.into_iter().map(|(px, py)| (-px, py)).collect();
    }
    let pts = rotate_pts(&pts, 0.0, 0.0, -rot);
    PadGeom::Poly { pts: pts.into_iter().map(|(px, py)| (x + px, y + py)).collect() }
}

/// Benutzerdefinierte Symbole (`symbols/<name>/features`): Name → Konturen am Ursprung (mm)
pub type UserSymbols = HashMap<String, Vec<Vec<Pt>>>;

/// Liest alle benutzerdefinierten Symbole eines Jobs.
pub fn parse_user_symbols(job: &OdbFiles) -> UserSymbols {
    let mut out = UserSymbols::new();
    for (path, content) in &job.files {
        let Some(rest) = path.strip_prefix("symbols/") else { continue };
        let Some(name) = rest.strip_suffix("/features") else { continue };
        if name.contains('/') {
            continue;
        }
        let f = parse_features(content, 0);
        let mut contours: Vec<Vec<Pt>> = f.surfaces;
        for p in f.pads {
            contours.push(match p.geom {
                PadGeom::Circle { d } => circle_pts(p.x, p.y, d / 2.0, 32),
                PadGeom::Rect { w, h } => rect_pts(p.x, p.y, w, h),
                PadGeom::Poly { pts } => pts,
            });
        }
        if !contours.is_empty() {
            out.insert(name.to_lowercase(), contours);
        }
    }
    out
}

pub fn parse_features(content: &str, max_prims: usize) -> Features {
    parse_features_ext(content, max_prims, &UserSymbols::new())
}

/// Wie `parse_features`, löst aber benutzerdefinierte Symbole über deren Konturen auf.
pub fn parse_features_ext(content: &str, max_prims: usize, user: &UserSymbols) -> Features {
    let mut f = Features::default();
    let mut sym_names: HashMap<String, String> = HashMap::new();
    let mut symbols: HashMap<String, Symbol> = HashMap::new();
    let mut is_inch = true;
    let mut scale = 25.4;
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut grow = |x0: f64, y0: f64, x1: f64, y1: f64| {
        b = [b[0].min(x0), b[1].min(y0), b[2].max(x1), b[3].max(y1)];
    };

    let mut in_surface = false;
    let mut surf_positive = true;
    let mut contour: Vec<Pt> = Vec::new();
    let mut contour_island = true;
    let mut last: Pt = (0.0, 0.0);

    let num = |s: Option<&&str>, sc: f64| -> Option<f64> { s.and_then(|v| v.parse::<f64>().ok()).map(|v| v * sc) };

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') || line.starts_with('&') {
            continue;
        }
        if let Some(inch) = units_line(line) {
            is_inch = inch;
            scale = if inch { 25.4 } else { 1.0 };
            continue;
        }
        if let Some(rest) = line.strip_prefix('$') {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 2 {
                let sym_inch = match parts.get(2).map(|u| u.to_uppercase()) {
                    Some(u) if u == "I" => true,
                    Some(u) if u == "M" => false,
                    _ => is_inch,
                };
                symbols.insert(parts[0].to_string(), parse_symbol(parts[1], sym_inch));
                sym_names.insert(parts[0].to_string(), parts[1].to_lowercase());
            }
            continue;
        }
        let data = strip_attrs(line);
        let p: Vec<&str> = data.split_whitespace().collect();
        if p.is_empty() {
            continue;
        }

        if in_surface {
            match p[0] {
                "OB" => {
                    contour.clear();
                    if let (Some(x), Some(y)) = (num(p.get(1), scale), num(p.get(2), scale)) {
                        contour.push((x, y));
                        last = (x, y);
                    }
                    contour_island = p.get(3).map_or(true, |t| !t.eq_ignore_ascii_case("H"));
                }
                "OS" => {
                    if let (Some(x), Some(y)) = (num(p.get(1), scale), num(p.get(2), scale)) {
                        contour.push((x, y));
                        last = (x, y);
                    }
                }
                "OC" => {
                    if let (Some(xe), Some(ye), Some(xc), Some(yc)) = (num(p.get(1), scale), num(p.get(2), scale), num(p.get(3), scale), num(p.get(4), scale)) {
                        let cw = p.get(5).map_or(true, |t| t.eq_ignore_ascii_case("Y"));
                        contour.extend(arc_points(last, (xe, ye), (xc, yc), cw));
                        last = (xe, ye);
                    }
                }
                "OE" => {
                    if contour.len() > 1 && contour.first() == contour.last() {
                        contour.pop();
                    }
                    if contour.len() >= 3 && contour_island && surf_positive {
                        let (x0, y0, x1, y1) = bounds(&contour);
                        grow(x0, y0, x1, y1);
                        if f.prims.len() < max_prims {
                            f.prims.push(Prim::poly(&contour));
                        }
                        f.surfaces.push(std::mem::take(&mut contour));
                    }
                    contour.clear();
                }
                "SE" => in_surface = false,
                _ => {}
            }
            continue;
        }

        match p[0] {
            "P" if p.len() >= 4 => {
                let (Some(x), Some(y)) = (num(p.get(1), scale), num(p.get(2), scale)) else { continue };
                let positive = p.get(4).map_or(true, |t| !t.eq_ignore_ascii_case("N"));
                let orient: i64 = p.get(6).and_then(|v| v.parse().ok()).unwrap_or(0);
                let (rot, mirror) = match orient {
                    0..=7 => ((orient % 4) as f64 * 90.0, orient >= 4),
                    8 | 9 => (p.get(7).and_then(|v| v.parse().ok()).unwrap_or(0.0), orient == 9),
                    _ => (0.0, false),
                };
                // Benutzersymbol: jede Kontur wird ein eigenes Pad (z. B. geteilte Pastenflächen)
                if let Some(contours) = sym_names.get(p[3]).and_then(|n| user.get(n)) {
                    for c in contours {
                        let pts: Vec<Pt> = if mirror { c.iter().map(|&(px, py)| (-px, py)).collect() } else { c.clone() };
                        let pts: Vec<Pt> = rotate_pts(&pts, 0.0, 0.0, -rot).into_iter().map(|(px, py)| (x + px, y + py)).collect();
                        let (x0, y0, x1, y1) = bounds(&pts);
                        grow(x0, y0, x1, y1);
                        if !positive {
                            continue;
                        }
                        if f.prims.len() < max_prims {
                            f.prims.push(Prim::poly(&pts));
                        }
                        f.pads.push(OdbPad { x: (x0 + x1) / 2.0, y: (y0 + y1) / 2.0, geom: PadGeom::Poly { pts } });
                    }
                    continue;
                }
                let sym = symbols.get(p[3]).cloned().unwrap_or(Symbol { shape: SymShape::Rect { r: 0.0 }, w: 0.5, h: 0.5 });
                let geom = place_pad(&sym, x, y, rot, mirror);
                let (x0, y0, x1, y1) = match &geom {
                    PadGeom::Circle { d } => (x - d / 2.0, y - d / 2.0, x + d / 2.0, y + d / 2.0),
                    PadGeom::Rect { w, h } => (x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0),
                    PadGeom::Poly { pts } => bounds(pts),
                };
                grow(x0, y0, x1, y1);
                if !positive {
                    continue;
                }
                if f.prims.len() < max_prims {
                    f.prims.push(match &geom {
                        PadGeom::Circle { d } => Prim::circle(x, y, d / 2.0),
                        PadGeom::Rect { w, h } => Prim::rect(x, y, *w, *h),
                        PadGeom::Poly { pts } => Prim::poly(pts),
                    });
                }
                f.pads.push(OdbPad { x, y, geom });
            }
            "L" if p.len() >= 6 => {
                let (Some(x0), Some(y0), Some(x1), Some(y1)) = (num(p.get(1), scale), num(p.get(2), scale), num(p.get(3), scale), num(p.get(4), scale)) else { continue };
                let w = symbols.get(p[5]).map_or(0.15, |s| s.w.min(s.h));
                grow(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
                let positive = p.get(6).map_or(true, |t| !t.eq_ignore_ascii_case("N"));
                if positive && f.prims.len() < max_prims {
                    f.prims.push(Prim::line(x0, y0, x1, y1, w));
                }
            }
            "A" if p.len() >= 8 => {
                let v: Vec<Option<f64>> = (1..=6).map(|i| num(p.get(i), scale)).collect();
                let (Some(x0), Some(y0), Some(x1), Some(y1), Some(xc), Some(yc)) = (v[0], v[1], v[2], v[3], v[4], v[5]) else { continue };
                let w = symbols.get(p[7]).map_or(0.15, |s| s.w.min(s.h));
                let positive = p.get(8).map_or(true, |t| !t.eq_ignore_ascii_case("N"));
                let cw = p.get(10).map_or(true, |t| t.eq_ignore_ascii_case("Y"));
                let pts = arc_points((x0, y0), (x1, y1), (xc, yc), cw);
                let mut prev = (x0, y0);
                for q in pts {
                    grow(prev.0.min(q.0), prev.1.min(q.1), prev.0.max(q.0), prev.1.max(q.1));
                    if positive && f.prims.len() < max_prims {
                        f.prims.push(Prim::line(prev.0, prev.1, q.0, q.1, w));
                    }
                    prev = q;
                }
            }
            "S" => {
                in_surface = true;
                surf_positive = p.get(1).map_or(true, |t| !t.eq_ignore_ascii_case("N"));
                contour.clear();
            }
            _ => {}
        }
    }
    if b[0].is_finite() {
        f.bbox = Some([r3(b[0]), r3(b[1]), r3(b[2]), r3(b[3])]);
    }
    f
}

// -------------------------------------------------------------
// EDA Packages & Bauteile
// -------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct EdaPin {
    pub pin: i64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Runde Pinfläche (CR-Record oder kreisförmige Kontur)
    pub round: bool,
}

#[derive(Debug, Clone)]
pub struct EdaPackage {
    pub name: String,
    pub pitch: f64,
    pub w: f64,
    pub h: f64,
    pub pins: Vec<EdaPin>,
}

pub fn parse_eda(content: &str) -> Vec<EdaPackage> {
    let mut pkgs: Vec<EdaPackage> = Vec::new();
    let mut scale = 25.4;
    let mut pin_pts: Vec<Pt> = Vec::new();
    let mut in_pin = false;
    let mut pin_arc = false;

    fn finish_pin(pkgs: &mut [EdaPackage], pts: &mut Vec<Pt>, arc: &mut bool) {
        if let (Some(pkg), false) = (pkgs.last_mut(), pts.is_empty()) {
            if let Some(pin) = pkg.pins.last_mut() {
                let (x0, y0, x1, y1) = bounds(pts);
                if x1 - x0 > 0.05 && y1 - y0 > 0.05 {
                    pin.w = r3(x1 - x0);
                    pin.h = r3(y1 - y0);
                    // Kontur aus Bögen mit quadratischer Hülle = Kreis
                    if *arc && ((pin.w - pin.h).abs() / pin.w.max(pin.h)) < 0.05 {
                        pin.round = true;
                    }
                }
            }
        }
        pts.clear();
        *arc = false;
    }

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inch) = units_line(line) {
            scale = if inch { 25.4 } else { 1.0 };
            continue;
        }
        let p: Vec<&str> = strip_attrs(line).split_whitespace().collect();
        if p.is_empty() {
            continue;
        }
        let f = |i: usize| p.get(i).and_then(|v| v.parse::<f64>().ok()).map(|v| v * scale);
        match p[0] {
            "PKG" if p.len() >= 7 => {
                if in_pin {
                    finish_pin(&mut pkgs, &mut pin_pts, &mut pin_arc);
                }
                in_pin = false;
                let (xmin, ymin, xmax, ymax) = (f(3).unwrap_or(0.0), f(4).unwrap_or(0.0), f(5).unwrap_or(0.0), f(6).unwrap_or(0.0));
                pkgs.push(EdaPackage {
                    name: p[1].to_string(),
                    pitch: r3(f(2).unwrap_or(0.0)),
                    w: r3((xmax - xmin).max(0.5)),
                    h: r3((ymax - ymin).max(0.5)),
                    pins: Vec::new(),
                });
            }
            "PIN" if p.len() >= 5 => {
                if in_pin {
                    finish_pin(&mut pkgs, &mut pin_pts, &mut pin_arc);
                }
                if let Some(pkg) = pkgs.last_mut() {
                    let idx = p[1].parse::<i64>().unwrap_or(pkg.pins.len() as i64 + 1);
                    pkg.pins.push(EdaPin { pin: idx, x: r3(f(3).unwrap_or(0.0)), y: r3(f(4).unwrap_or(0.0)), w: 0.8, h: 0.8, round: false });
                    in_pin = true;
                }
            }
            "RC" if in_pin && p.len() >= 5 => {
                if let (Some(w), Some(h), Some(pkg)) = (f(3), f(4), pkgs.last_mut()) {
                    if let Some(pin) = pkg.pins.last_mut() {
                        if w.abs() > 0.05 && h.abs() > 0.05 {
                            pin.w = r3(w.abs());
                            pin.h = r3(h.abs());
                        }
                    }
                }
            }
            "CR" if in_pin && p.len() >= 4 => {
                if let (Some(r), Some(pkg)) = (f(3), pkgs.last_mut()) {
                    if let Some(pin) = pkg.pins.last_mut() {
                        if r > 0.02 {
                            pin.w = r3(2.0 * r);
                            pin.h = r3(2.0 * r);
                            pin.round = true;
                        }
                    }
                }
            }
            "SQ" if in_pin && p.len() >= 4 => {
                if let (Some(hs), Some(pkg)) = (f(3), pkgs.last_mut()) {
                    if let Some(pin) = pkg.pins.last_mut() {
                        pin.w = r3(2.0 * hs);
                        pin.h = r3(2.0 * hs);
                    }
                }
            }
            "OB" | "OS" | "OC" if in_pin => {
                if p[0] == "OC" {
                    pin_arc = true;
                }
                if let (Some(x), Some(y)) = (f(1), f(2)) {
                    pin_pts.push((x, y));
                }
            }
            "FGR" | "PRP" | "NET" | "SNT" | "FID" => {
                if in_pin {
                    finish_pin(&mut pkgs, &mut pin_pts, &mut pin_arc);
                }
                in_pin = false;
            }
            _ => {}
        }
    }
    if in_pin {
        finish_pin(&mut pkgs, &mut pin_pts, &mut pin_arc);
    }
    pkgs
}

#[derive(Debug, Clone)]
pub struct OdbComponent {
    pub refdes: String,
    pub pkg_ref: i64,
    pub x: f64,
    pub y: f64,
    pub rot: f64,
    pub mirror: bool,
    /// Toeprints: (Pin-Nummer 1-basiert, x, y) in Board-Koordinaten
    pub pins: Vec<(i64, f64, f64)>,
}

pub fn parse_components(content: &str) -> Vec<OdbComponent> {
    let mut out: Vec<OdbComponent> = Vec::new();
    let mut scale = 25.4;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inch) = units_line(line) {
            scale = if inch { 25.4 } else { 1.0 };
            continue;
        }
        let p: Vec<&str> = strip_attrs(line).split_whitespace().collect();
        let f = |i: usize| p.get(i).and_then(|v| v.parse::<f64>().ok());
        match p.first().copied() {
            Some("CMP") if p.len() >= 6 => {
                let refdes = p.get(6).unwrap_or(&p[5]).to_string();
                out.push(OdbComponent {
                    refdes,
                    pkg_ref: p[1].parse().unwrap_or(-1),
                    x: f(2).unwrap_or(0.0) * scale,
                    y: f(3).unwrap_or(0.0) * scale,
                    rot: f(4).unwrap_or(0.0),
                    mirror: p.get(5).map_or(false, |m| m.eq_ignore_ascii_case("M")),
                    pins: Vec::new(),
                });
            }
            Some("TOP") if p.len() >= 4 => {
                if let (Some(c), Some(pin), Some(x), Some(y)) = (out.last_mut(), p[1].parse::<i64>().ok(), f(2), f(3)) {
                    c.pins.push((pin + 1, x * scale, y * scale));
                }
            }
            _ => {}
        }
    }
    out
}

// -------------------------------------------------------------
// UNIX compress (.Z, LZW) – ODB++ speichert Features häufig komprimiert
// -------------------------------------------------------------
pub fn uncompress_z(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 3 || data[0] != 0x1f || data[1] != 0x9d {
        return None;
    }
    let maxbits = (data[2] & 0x1f) as u32;
    let block_mode = data[2] & 0x80 != 0;
    if !(9..=16).contains(&maxbits) {
        return None;
    }
    let buf = &data[3..];
    let total_bits = buf.len() * 8;
    let maxmaxcode: u32 = 1 << maxbits;
    let mut n_bits: u32 = 9;
    let mut maxcode: u32 = (1 << n_bits) - 1;
    let mut bitmask: u32 = (1 << n_bits) - 1;
    let mut prefix = vec![0u32; maxmaxcode as usize];
    let mut suffix = vec![0u8; maxmaxcode as usize];
    for i in 0..256 {
        suffix[i] = i as u8;
    }
    let mut free_ent: u32 = if block_mode { 257 } else { 256 };
    let mut oldcode: i64 = -1;
    let mut finchar: u8 = 0;
    let mut posbits: usize = 0;
    let mut out = Vec::with_capacity(buf.len() * 3);
    let mut stack: Vec<u8> = Vec::with_capacity(1 << 16);
    let align = |posbits: usize, n_bits: u32| -> usize {
        let g = (n_bits as usize) << 3;
        (posbits - 1) + (g - (posbits - 1 + g) % g)
    };

    while posbits + n_bits as usize <= total_bits {
        if free_ent > maxcode {
            posbits = align(posbits, n_bits);
            n_bits += 1;
            maxcode = if n_bits == maxbits { maxmaxcode } else { (1 << n_bits) - 1 };
            bitmask = (1 << n_bits) - 1;
            continue;
        }
        let byte = posbits >> 3;
        let mut word: u32 = 0;
        for k in 0..3 {
            word |= (*buf.get(byte + k).unwrap_or(&0) as u32) << (8 * k);
        }
        let mut code = (word >> (posbits & 7)) & bitmask;
        posbits += n_bits as usize;

        if oldcode == -1 {
            if code >= 256 {
                return None;
            }
            finchar = code as u8;
            oldcode = code as i64;
            out.push(finchar);
            continue;
        }
        if code == 256 && block_mode {
            free_ent = 256;
            posbits = align(posbits, n_bits);
            n_bits = 9;
            maxcode = (1 << n_bits) - 1;
            bitmask = (1 << n_bits) - 1;
            continue;
        }
        let incode = code;
        stack.clear();
        if code >= free_ent {
            if code > free_ent {
                return None;
            }
            stack.push(finchar);
            code = oldcode as u32;
        }
        while code >= 256 {
            stack.push(suffix[code as usize]);
            code = prefix[code as usize];
        }
        finchar = suffix[code as usize];
        stack.push(finchar);
        out.extend(stack.iter().rev());
        if free_ent < maxmaxcode {
            prefix[free_ent as usize] = oldcode as u32;
            suffix[free_ent as usize] = finchar;
            free_ent += 1;
        }
        oldcode = incode as i64;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_units() {
        let s = parse_symbol("rect30.5118x7.874xr3.1496", true);
        assert!((s.w - 0.775).abs() < 1e-3 && (s.h - 0.2).abs() < 1e-3);
        let c = parse_symbol("r19.685", true);
        assert!(matches!(c.shape, SymShape::Circle) && (c.w - 0.5).abs() < 1e-3);
        let m = parse_symbol("r500", false);
        assert!((m.w - 0.5).abs() < 1e-9);
    }

    #[test]
    fn features_rotation_and_attributes() {
        let feat = "UNITS=MM\n$0 rect1000x500\n$1 r400\nP 10 20 0 P 0 1;0=0\nP 5 5 1 P 0 0\nP 1 1 0 N 0 0\nS P 0\nOB 0 0 I\nOS 1 0\nOS 1 1\nOS 0 1\nOE\nSE\n";
        let f = parse_features(feat, 1000);
        assert_eq!(f.pads.len(), 2);
        match f.pads[0].geom {
            PadGeom::Rect { w, h } => assert!((w - 0.5).abs() < 1e-9 && (h - 1.0).abs() < 1e-9),
            _ => panic!("Rechteck erwartet"),
        }
        assert_eq!(f.surfaces.len(), 1);
    }

    #[test]
    fn eda_and_components() {
        let eda = "UNITS=MM\nPKG 0805 1.7 -1.5 -0.7 1.5 0.7;;ID=1\nRC -1.5 -0.7 3 1.4\nPIN 1 S -0.85 0 0 U U\nRC -1.45 -0.65 1.2 1.3\nPIN 2 S 0.85 0 0 U U\nRC 0.25 -0.65 1.2 1.3\n";
        let pk = parse_eda(eda);
        assert_eq!(pk.len(), 1);
        assert_eq!(pk[0].pins.len(), 2);
        assert!((pk[0].pins[1].w - 1.2).abs() < 1e-9);
        let comps = parse_components("UNITS=MM\nCMP 0 10 10 90 N R1 RES;0=1\nTOP 0 9.15 10 0 N 1 0 1\nTOP 1 10.85 10 0 N 2 0 2\n");
        assert_eq!(comps[0].refdes, "R1");
        assert_eq!(comps[0].pins.len(), 2);
        assert_eq!(comps[0].pins[1].0, 2);
    }

    #[test]
    fn matrix_blocks() {
        let m = "STEP {\n COL=1\n NAME=PCB\n}\nLAYER {\n ROW=2\n CONTEXT=BOARD\n TYPE=SOLDER_PASTE\n NAME=TOP_PASTE\n}\nLAYER {\n ROW=1\n TYPE=SIGNAL\n NAME=TOP\n}\n";
        let l = parse_matrix(m);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].name, "top");
        assert_eq!(l[1].ltype, "SOLDER_PASTE");
    }
}
