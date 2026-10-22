//! Regelbasierte Apertur-Geometrie (Simple Rectangle, HouseBuilder, Inverted House, Chamfer,
//! Länglich & Schmal, Array Slicing orthogonal und 45°-diagonal).

use super::geometry::{bounds, centroid, circle_pts, clip_halfplane, convex_hull, fillet_polygon, inset_convex, polygon_area, rect_pts, rounded_rect_pts};
use super::donut_segments;
use super::types::{r4, r5, round_pts, PackageRule, Pt, Slice};
use serde_json::{Map, Value};

fn poly_slice(cx: f64, cy: f64, w: f64, h: f64, pts: &[Pt], kind: &str) -> Slice {
    Slice {
        cx: r4(cx),
        cy: r4(cy),
        w: r4(w),
        h: r4(h),
        shape: Some("polygon".into()),
        kind: Some(kind.into()),
        pts: Some(round_pts(pts)),
        ..Default::default()
    }
}

/// Rechteckige Öffnung, bei Radius > 0 mit echten Bogenecken.
fn rounded_slice(cx: f64, cy: f64, w: f64, h: f64, r: f64, kind: &str) -> Slice {
    let r = r.min(w / 2.2).min(h / 2.2).max(0.0);
    if r > 0.005 {
        let mut s = poly_slice(cx, cy, w, h, &rounded_rect_pts(cx, cy, w, h, r), kind);
        s.radius = Some(r4(r));
        s
    } else {
        rect_slice(cx, cy, w, h, Some(kind))
    }
}

/// "Take Existing" bei rechteckiger Kundenpaste: gleiche Größe, Ecken mit dem Eckradius der Regel
/// abgerundet (wie PCB Investigator); Radius 0 lässt das Original unverändert.
pub fn take_existing_rounded(rule: &PackageRule, rectangle: bool, cx: f64, cy: f64, w: f64, h: f64) -> Option<Vec<Slice>> {
    (rule.rule_type == "take_existing" && rectangle && rule.corner_radius > 0.005).then(|| vec![rounded_slice(cx, cy, w, h, rule.corner_radius, "take_existing")])
}

/// Schräge oder nicht rechteckige Pads (das Polygon füllt sein umschließendes Rechteck nicht):
/// Die Regelformen sind achsparallel, daher wird die echte Kontur um das Undersize nach innen versetzt.
/// Abgerundete Rechtecke (Füllgrad ≥ 95 %) laufen weiter über die normale Regel.
pub fn shape_true_opening(pts: &[Pt], rule: &PackageRule) -> Option<Vec<Slice>> {
    if pts.len() < 3 || matches!(rule.rule_type.as_str(), "take_existing" | "ignore") {
        return None;
    }
    let (x0, y0, x1, y1) = bounds(pts);
    let area = polygon_area(pts);
    if area >= 0.95 * ((x1 - x0) * (y1 - y0)).max(1e-12) {
        return None;
    }
    // Nicht konvexe Sonderformen unverändert lassen, das Versetzen über die Hülle würde sie füllen
    let convex = area >= 0.98 * polygon_area(&convex_hull(pts));
    let inset = if convex { inset_convex(pts, rule.undersize_w.min(rule.undersize_h).max(0.0)) } else { Vec::new() };
    let poly = if inset.len() >= 3 { inset } else { pts.to_vec() };
    let (a, b, c, d) = bounds(&poly);
    Some(vec![poly_slice((a + c) / 2.0, (b + d) / 2.0, c - a, d - b, &poly, "shape_true")])
}

/// Haus-Öffnung; Ecken und Spitze mit dem Eckradius abgerundet (wie PCB Investigator).
fn house_slice(cx: f64, cy: f64, w: f64, h: f64, pts: &[Pt], r: f64, kind: &str) -> Slice {
    let r = r.min(w / 2.2).min(h / 2.2).max(0.0);
    let mut s = poly_slice(cx, cy, w, h, &fillet_polygon(pts, r, 6), kind);
    if r > 0.005 {
        s.radius = Some(r4(r));
    }
    s
}

fn circle_slice(cx: f64, cy: f64, d: f64, kind: &str) -> Slice {
    Slice {
        cx: r4(cx),
        cy: r4(cy),
        w: r4(d),
        h: r4(d),
        shape: Some("circle".into()),
        kind: Some(kind.into()),
        pts: Some(round_pts(&circle_pts(cx, cy, d / 2.0, 36))),
        ..Default::default()
    }
}

/// Gitter aus Teilöffnungen (gx × gy) mit Stegbreite `web`, Ecken gerundet.
fn grid_slices(cx: f64, cy: f64, w: f64, h: f64, gx: usize, gy: usize, web: f64, r: f64, kind: &str) -> Option<Vec<Slice>> {
    let sub_w = (w - (gx as f64 - 1.0) * web) / gx as f64;
    let sub_h = (h - (gy as f64 - 1.0) * web) / gy as f64;
    if sub_w <= 0.0 || sub_h <= 0.0 {
        return None;
    }
    let start_x = cx - w / 2.0 + sub_w / 2.0;
    let start_y = cy - h / 2.0 + sub_h / 2.0;
    let mut out = Vec::with_capacity(gx * gy);
    for ix in 0..gx {
        for iy in 0..gy {
            out.push(rounded_slice(start_x + ix as f64 * (sub_w + web), start_y + iy as f64 * (sub_h + web), sub_w, sub_h, r, kind));
        }
    }
    Some(out)
}

/// Array-Felder für große Pads: Feldanzahl so, dass jedes Feld ≥ `min_field` ist und der Abstand
/// `gap` eingehalten wird; Feldgröße so, dass die Gesamtöffnung `util` (0..1) der Padfläche ergibt.
pub fn array_fields(cx: f64, cy: f64, w: f64, h: f64, min_field: f64, gap: f64, util: f64, r: f64) -> Vec<Slice> {
    let nx = (((w + gap) / (min_field + gap)).floor() as usize).max(1);
    let ny = (((h + gap) / (min_field + gap)).floor() as usize).max(1);
    let (px, py) = (w / nx as f64, h / ny as f64);
    let s = util.clamp(0.05, 1.0).sqrt();
    let fw = (px * s).min(px - gap).max(min_field.min(px));
    let fh = (py * s).min(py - gap).max(min_field.min(py));
    let mut out = Vec::with_capacity(nx * ny);
    for ix in 0..nx {
        for iy in 0..ny {
            let fx = cx - w / 2.0 + px * (ix as f64 + 0.5);
            let fy = cy - h / 2.0 + py * (iy as f64 + 0.5);
            out.push(rounded_slice(fx, fy, fw, fh, r, "array_fields"));
        }
    }
    out
}

fn rect_slice(cx: f64, cy: f64, w: f64, h: f64, kind: Option<&str>) -> Slice {
    Slice {
        cx: r4(cx),
        cy: r4(cy),
        w: r4(w),
        h: r4(h),
        shape: Some("rectangle".into()),
        kind: kind.map(|k| k.to_string()),
        ..Default::default()
    }
}

/// Löst eine Pin-spezifische Übersteuerung gegen die Package-Basisregel auf.
pub fn resolve_pin_rule(base: &PackageRule, pin_data: &Map<String, Value>) -> PackageRule {
    let mut rtype = pin_data
        .get("rule_type")
        .and_then(|v| v.as_str())
        .unwrap_or("inherit")
        .to_string();
    if rtype.is_empty() || rtype == "inherit" {
        rtype = base.rule_type.clone();
    }
    let mut rule = base.clone();
    let mut data = pin_data.clone();
    data.remove("rule_type");
    data.remove("name");
    rule.apply_map(&data);
    rule.rule_type = rtype.clone();
    rule.name = rtype;
    rule
}

/// Geometrischer Kontext eines Kupferpads für die Regelanwendung.
#[derive(Debug, Clone, Copy, Default)]
pub struct PadCtx<'a> {
    pub w: f64,
    pub h: f64,
    pub cx: f64,
    pub cy: f64,
    pub facing: &'a str,
    /// Lötstopp-Freistellung (für Pin-minus-Mask)
    pub mask: Option<(f64, f64)>,
    pub round: bool,
    /// Mittenabstand zum Nachbarpad quer zur Padlänge (Pitch), falls bekannt
    pub pitch: Option<f64>,
    /// Mindest-Stegbreite im Schablonenblech
    pub min_bridge: f64,
    /// Anzahl Pins des Packages (für die Regel "Package-Gruppe")
    pub pin_count: usize,
    /// Bohrdurchmesser bei THT-Pads (für Donut / Pin-in-Paste)
    pub drill: Option<f64>,
    /// Effektive Blechdicke am Pad (mm, für Pin-in-Paste)
    pub thickness: f64,
    /// Kupferpad (B, H, rund) – die Regel "Package-Gruppe" rechnet wie PCB Investigator vom Kupfer aus
    pub copper: Option<(f64, f64, bool)>,
    /// Padmitte relativ zum Bauteilzentrum (im selben Rahmen wie cx/cy)
    pub rel: Option<(f64, f64)>,
    /// Lage des Bauteils: Drehung im Uhrzeigersinn (Grad) und Spiegelung (BOT) vom Package- in den Pad-Rahmen
    pub rot: f64,
    pub mirror: bool,
}

/// Feste Größe (B, H) der Regel im Rahmen des Pads: bei um 90°/270° gedrehten Bauteilen getauscht.
pub fn fixed_size(rule: &PackageRule, pad: &PadCtx) -> (f64, f64) {
    if (pad.rot.rem_euclid(180.0) - 90.0).abs() < 1.0 { (rule.size_h, rule.size_w) } else { (rule.size_w, rule.size_h) }
}

/// Vektor vom Package-Rahmen in den Rahmen des Pads (ODB++: erst im Uhrzeigersinn drehen, dann bei BOT spiegeln).
pub fn pkg_to_frame(v: (f64, f64), rot: f64, mirror: bool) -> (f64, f64) {
    let (s, c) = rot.to_radians().sin_cos();
    let (x, y) = (v.0 * c + v.1 * s, -v.0 * s + v.1 * c);
    if mirror { (-x, y) } else { (x, y) }
}

/// Umkehrung von `pkg_to_frame`.
pub fn frame_to_pkg(v: (f64, f64), rot: f64, mirror: bool) -> (f64, f64) {
    let (x, y) = if mirror { (-v.0, v.1) } else { v };
    let (s, c) = rot.to_radians().sin_cos();
    (x * c - y * s, x * s + y * c)
}

/// Einheitsvektor "nach außen" für ein Pad gemäß `move_axis` (im Rahmen des Pads).
pub fn outward_vec(pad: &PadCtx, axis: &str) -> (f64, f64) {
    let from_facing = || match pad.facing {
        "right" => (-1.0, 0.0),
        "left" => (1.0, 0.0),
        "up" => (0.0, -1.0),
        "down" => (0.0, 1.0),
        _ => (0.0, 0.0),
    };
    let sign = |v: f64| if v > 1e-6 { 1.0 } else if v < -1e-6 { -1.0 } else { 0.0 };
    let Some(rel) = pad.rel else { return from_facing() };
    match axis {
        "long" => {
            // Längsachse im Pad-Rahmen; ohne Versatz in dieser Achse (Pad mittig) Richtung zum Zentrum
            let v = if pad.w >= pad.h { (sign(rel.0), 0.0) } else { (0.0, sign(rel.1)) };
            if v == (0.0, 0.0) { from_facing() } else { v }
        }
        "x" | "y" => {
            let p = frame_to_pkg(rel, pad.rot, pad.mirror);
            let v = if axis == "x" { (sign(p.0), 0.0) } else { (0.0, sign(p.1)) };
            pkg_to_frame(v, pad.rot, pad.mirror)
        }
        _ => from_facing(),
    }
}

impl<'a> PadCtx<'a> {
    pub fn new(w: f64, h: f64, cx: f64, cy: f64, facing: &'a str) -> Self {
        PadCtx { w, h, cx, cy, facing, ..Default::default() }
    }
}

/// Breite der Pastenöffnung quer zur Padlänge für "Länglich & Schmal".
/// Grundsätzlich `Padbreite × (1 − Reduktion)`. Sind die Pads breiter als der Kupferabstand
/// (Fine-Pitch-Stecker, ICs), wird zusätzlich so weit verschmälert, dass der Abstand zwischen
/// den Pastenöffnungen mindestens der Kupfer-Padbreite entspricht – und nie unter die
/// Mindest-Stegbreite fällt.
pub fn elongated_narrow_width(narrow: f64, reduction_pct: f64, pitch: Option<f64>, min_bridge: f64, gap_ratio: f64) -> f64 {
    let mut n = narrow * (1.0 - reduction_pct / 100.0);
    if let Some(p) = pitch.filter(|p| *p > narrow * 0.5) {
        // Pastenabstand = gap_ratio × Öffnungsbreite → Breite = Pitch / (1 + gap_ratio)
        n = n.min(p / (1.0 + gap_ratio.max(0.2)));
        if min_bridge > 0.0 {
            n = n.min(p - min_bridge);
        }
    }
    n.max(0.10_f64.min(narrow))
}

/// Länge für "Länglich & Schmal": Längennutzung plus `comp_pct` % der relativen Verschmälerung.
pub fn elongated_length(len: f64, narrow: f64, new_narrow: f64, usage_pct: f64, comp_pct: f64) -> f64 {
    let shrink = if narrow > 0.0 { (1.0 - new_narrow / narrow).max(0.0) } else { 0.0 };
    len * (usage_pct / 100.0 + shrink * comp_pct / 100.0)
}

/// Erzeugt die Schablonenöffnung(en) für ein Pad nach einer Package-Regel.
/// `None` = Original übernehmen (take_existing), leere Liste = Pad ignorieren.
pub fn generate_rule_aperture_geometry(pad: &PadCtx, rule: &PackageRule) -> Option<Vec<Slice>> {
    let (pad_w, pad_h, pad_cx, pad_cy, facing_dir, round) = (pad.w, pad.h, pad.cx, pad.cy, pad.facing, pad.round);
    let (mask_w, mask_h) = pad.mask.map(|(a, b)| (Some(a), Some(b))).unwrap_or((None, None));
    let rtype = rule.rule_type.as_str();
    if rtype == "take_existing" {
        return None;
    }
    if rtype == "ignore" {
        return Some(Vec::new());
    }

    // Pin-minus-Mask (Solder Mask Defined Pads)
    let (eff_pw, eff_ph) = match (rule.pin_minus_mask, mask_w, mask_h) {
        (true, Some(mw), Some(mh)) if mw > 0.0 && mh > 0.0 => (pad_w.min(mw), pad_h.min(mh)),
        _ => (pad_w, pad_h),
    };

    let w_eff = (eff_pw - 2.0 * rule.undersize_w).max(0.04);
    let h_eff = (eff_ph - 2.0 * rule.undersize_h).max(0.04);

    // Offset X/Y gilt im Package-Rahmen und dreht mit dem Bauteil
    let off = pkg_to_frame((rule.offset_x, rule.offset_y), pad.rot, pad.mirror);
    let mut cx = pad_cx + off.0;
    let mut cy = pad_cy + off.1;

    // Move away und Toe-/Heel-Shift (>0 nach außen / Toe, <0 nach innen / Heel)
    let out = outward_vec(pad, &rule.move_axis);
    let away = rule.offset_move_away + rule.toe_heel_shift;
    if away != 0.0 {
        cx += out.0 * away;
        cy += out.1 * away;
    }

    // Runde Pads (Stecker, THT, BGA) bleiben rund: Undersize/Offsets wirken auf den Durchmesser.
    // Rechteck-spezifische Formen (House, Chamfer, Länglich) sind für Kreise nicht sinnvoll.
    // Regel "Package-Gruppe": wählt je Pad die passende Form (PCB Investigator "Inherit")
    if rtype == "package_group" {
        // Basis ist das Kupferpad (wie PCB Investigator), nicht die Pastenvorgabe des Kunden
        if let Some((cw, ch, cround)) = pad.copper.filter(|c| c.0 > 0.0 && c.1 > 0.0) {
            let sub = PadCtx { w: cw, h: ch, round: cround, ..*pad };
            let (w, h) = ((cw - 2.0 * rule.undersize_w).max(0.04), (ch - 2.0 * rule.undersize_h).max(0.04));
            return Some(package_group_geometry(&sub, rule, cx, cy, w, h));
        }
        return Some(package_group_geometry(pad, rule, cx, cy, w_eff, h_eff));
    }
    if rtype == "pin_in_paste" {
        return Some(pin_in_paste_geometry(pad, rule, cx, cy, w_eff, h_eff));
    }
    if rtype == "donut" {
        // Ringöffnung um die Bohrung, segmentiert mit Haltestegen (Bridge Width)
        let d_out = w_eff.min(h_eff);
        let d_in = if rule.donut_inner_dia > 0.0 { rule.donut_inner_dia } else { pad.drill.map_or(d_out * 0.5, |d| d + 0.1) };
        if d_in >= d_out * 0.95 {
            return Some(vec![circle_slice(cx, cy, d_out, "donut")]);
        }
        return Some(donut_segments(cx, cy, d_in, d_out, rule.bridge_width.max(0.1)));
    }
    if round && rtype != "array_slicing" && rtype != "array_pads" {
        let d = w_eff.min(h_eff);
        return Some(vec![Slice {
            cx: r4(cx),
            cy: r4(cy),
            w: r4(d),
            h: r4(d),
            shape: Some("circle".into()),
            kind: Some(rtype.to_string()),
            pts: Some(round_pts(&circle_pts(cx, cy, d / 2.0, 36))),
            ..Default::default()
        }]);
    }

    // 45°-Eck-Anfasung
    if rule.chamfer_size > 0.005 && (rtype == "simple_rect" || rtype == "chamfered_rect") {
        let c = rule.chamfer_size.min(w_eff / 2.1).min(h_eff / 2.1);
        let (hw, hh) = (w_eff / 2.0, h_eff / 2.0);
        let mode = if rule.chamfer_mode.is_empty() { "all" } else { rule.chamfer_mode.as_str() };
        let f = facing_dir;
        let tl = mode == "all" || mode == "pin1" || (mode == "toe" && (f == "left" || f == "up")) || (mode == "heel" && (f == "right" || f == "down"));
        let tr = mode == "all" || (mode == "toe" && (f == "right" || f == "up")) || (mode == "heel" && (f == "left" || f == "down"));
        let br = mode == "all" || (mode == "toe" && (f == "right" || f == "down")) || (mode == "heel" && (f == "left" || f == "up"));
        let bl = mode == "all" || (mode == "toe" && (f == "left" || f == "down")) || (mode == "heel" && (f == "right" || f == "up"));

        let mut pts = Vec::with_capacity(8);
        if bl {
            pts.push((cx - hw + c, cy - hh));
            pts.push((cx - hw, cy - hh + c));
        } else {
            pts.push((cx - hw, cy - hh));
        }
        if tl {
            pts.push((cx - hw, cy + hh - c));
            pts.push((cx - hw + c, cy + hh));
        } else {
            pts.push((cx - hw, cy + hh));
        }
        if tr {
            pts.push((cx + hw - c, cy + hh));
            pts.push((cx + hw, cy + hh - c));
        } else {
            pts.push((cx + hw, cy + hh));
        }
        if br {
            pts.push((cx + hw, cy - hh + c));
            pts.push((cx + hw - c, cy - hh));
        } else {
            pts.push((cx + hw, cy - hh));
        }
        let mut s = poly_slice(cx, cy, w_eff, h_eff, &pts, "chamfered_rect");
        s.radius = Some(0.0);
        return Some(vec![s]);
    }

    match rtype {
        "simple_rect" => {
            let (fw, fh) = fixed_size(rule, pad);
            let w = if fw > 0.0 { fw } else { w_eff };
            let h = if fh > 0.0 { fh } else { h_eff };
            if rule.bridge_count_x > 0 || rule.bridge_count_y > 0 {
                if rule.rotate_bridges_45 {
                    let d = diagonal_slices(cx, cy, w, h, rule);
                    if !d.is_empty() {
                        return Some(d);
                    }
                }
                let gx = (rule.bridge_count_x + 1) as usize;
                let gy = (rule.bridge_count_y + 1) as usize;
                if let Some(g) = grid_slices(cx, cy, w, h, gx, gy, rule.bridge_width.max(0.08), rule.corner_radius, "simple_rect") {
                    return Some(g);
                }
            }
            Some(vec![rounded_slice(cx, cy, w, h, rule.corner_radius, "simple_rect")])
        }
        "house_builder" | "inverted_house_builder" if rule.size_w > 0.0 || rule.size_h > 0.0 => {
            // Feste Größe (PCB-I "FixedWidth/Height"): Haus mit vorgegebenen Maßen
            let mut r = rule.clone();
            r.size_w = 0.0;
            r.size_h = 0.0;
            r.undersize_w = 0.0;
            r.undersize_h = 0.0;
            r.offset_x = 0.0;
            r.offset_y = 0.0;
            r.offset_move_away = 0.0;
            r.toe_heel_shift = 0.0;
            let (fw, fh) = fixed_size(rule, pad);
            let w = if fw > 0.0 { fw } else { w_eff };
            let h = if fh > 0.0 { fh } else { h_eff };
            let sub = PadCtx { w, h, cx, cy, mask: None, ..*pad };
            generate_rule_aperture_geometry(&sub, &r)
        }
        "house_builder" => {
            let cut_w = w_eff * (rule.house_cut_pct / 100.0);
            let cut_h = h_eff * (rule.house_cut_pct / 100.0);
            let (hw, hh) = (w_eff / 2.0, h_eff / 2.0);
            let pts = match facing_dir {
                "right" => vec![(cx - hw, cy - hh), (cx + hw - cut_w, cy - hh), (cx + hw, cy), (cx + hw - cut_w, cy + hh), (cx - hw, cy + hh)],
                "left" => vec![(cx - hw + cut_w, cy - hh), (cx + hw, cy - hh), (cx + hw, cy + hh), (cx - hw + cut_w, cy + hh), (cx - hw, cy)],
                "down" => vec![(cx - hw, cy + hh), (cx + hw, cy + hh), (cx + hw, cy - hh + cut_h), (cx, cy - hh), (cx - hw, cy - hh + cut_h)],
                _ => vec![(cx - hw, cy - hh), (cx - hw, cy + hh - cut_h), (cx, cy + hh), (cx + hw, cy + hh - cut_h), (cx + hw, cy - hh)],
            };
            Some(vec![house_slice(cx, cy, w_eff, h_eff, &pts, rule.corner_radius, "house_builder")])
        }
        "inverted_house_builder" => {
            let cut_w = w_eff * (rule.house_cut_pct / 100.0);
            let cut_h = h_eff * (rule.house_cut_pct / 100.0);
            let (hw, hh) = (w_eff / 2.0, h_eff / 2.0);
            let pts = match facing_dir {
                "right" => vec![(cx + hw, cy - hh), (cx - hw + cut_w, cy - hh), (cx - hw, cy), (cx - hw + cut_w, cy + hh), (cx + hw, cy + hh)],
                "left" => vec![(cx - hw, cy - hh), (cx + hw - cut_w, cy - hh), (cx + hw, cy), (cx + hw - cut_w, cy + hh), (cx - hw, cy + hh)],
                "up" => vec![(cx - hw, cy + hh), (cx + hw, cy + hh), (cx + hw, cy - hh + cut_h), (cx, cy - hh), (cx - hw, cy - hh + cut_h)],
                _ => vec![(cx - hw, cy - hh), (cx - hw, cy + hh - cut_h), (cx, cy + hh), (cx + hw, cy + hh - cut_h), (cx + hw, cy - hh)],
            };
            Some(vec![house_slice(cx, cy, w_eff, h_eff, &pts, rule.corner_radius, "inverted_house_builder")])
        }
        "elongated_pitch" => {
            let (new_w, new_h) = if pad_w <= pad_h {
                let n = elongated_narrow_width(pad_w, rule.pitch_reduction_pct, pad.pitch, pad.min_bridge, rule.gap_ratio);
                (n, elongated_length(pad_h, pad_w, n, rule.length_usage_pct, rule.length_comp_pct))
            } else {
                let n = elongated_narrow_width(pad_h, rule.pitch_reduction_pct, pad.pitch, pad.min_bridge, rule.gap_ratio);
                (elongated_length(pad_w, pad_h, n, rule.length_usage_pct, rule.length_comp_pct), n)
            };
            // Versatz nach außen (weg vom Bauteilzentrum)
            let (cx, cy) = (cx + out.0 * rule.outward_shift, cy + out.1 * rule.outward_shift);
            let r = rule.corner_radius.min(new_w / 2.5).min(new_h / 2.5).max(0.0);
            let mut s = poly_slice(cx, cy, new_w, new_h, &rounded_rect_pts(cx, cy, new_w, new_h, r), "elongated_pitch");
            s.radius = Some(r4(r));
            Some(vec![s])
        }
        "array_slicing" | "array_pads" => {
            if rule.rotate_bridges_45 {
                let slices = diagonal_slices(cx, cy, w_eff, h_eff, rule);
                if !slices.is_empty() {
                    return Some(slices);
                }
            }
            let gx = (rule.bridge_count_x + 1).max(1) as usize;
            let gy = (rule.bridge_count_y + 1).max(1) as usize;
            Some(grid_slices(cx, cy, w_eff, h_eff, gx, gy, rule.bridge_width.max(0.08), rule.corner_radius, "array_slicing")
                .unwrap_or_else(|| vec![rect_slice(cx, cy, w_eff, h_eff, None)]))
        }
        _ => Some(vec![rounded_slice(cx, cy, w_eff, h_eff, rule.corner_radius, "simple_rect")]),
    }
}

/// Erforderliche Öffnungsfläche (mm²) für Pin-in-Paste nach IPC-7525:
/// (Bohrungsvolumen − Pinvolumen + 20 % Meniskus) / Metallanteil / Blechdicke.
pub fn pip_required_area(drill: f64, pin_d: f64, pcb: f64, metal_pct: f64, thickness: f64) -> f64 {
    use std::f64::consts::PI;
    let v_hole = ((PI * drill * drill / 4.0 - PI * pin_d * pin_d / 4.0) * pcb).max(0.0);
    let v_paste = v_hole * 1.2 / (metal_pct / 100.0).clamp(0.1, 1.0);
    if thickness > 0.0 { v_paste / thickness } else { 0.0 }
}

/// Pin-in-Paste: Öffnungsgröße automatisch aus Bohrung, Leiterplattendicke, Metallanteil und Blechdicke;
/// begrenzt durch maximale Größe bzw. Pitch − Mindeststeg, mindestens Padgröße.
fn pin_in_paste_geometry(pad: &PadCtx, rule: &PackageRule, cx: f64, cy: f64, w_eff: f64, h_eff: f64) -> Vec<Slice> {
    use std::f64::consts::PI;
    let Some(drill) = pad.drill.filter(|d| *d > 0.05) else {
        // Kein Bohrloch bekannt: Pad 1:1 (rund bleibt rund)
        return if pad.round { vec![circle_slice(cx, cy, w_eff.min(h_eff), "pin_in_paste")] } else { vec![rounded_slice(cx, cy, w_eff, h_eff, rule.corner_radius, "pin_in_paste")] };
    };
    let t = if pad.thickness > 0.0 { pad.thickness } else { 0.12 };
    let pin_d = (drill - rule.pip_pin_clearance).max(0.1);
    let area = pip_required_area(drill, pin_d, rule.pip_pcb_thickness, rule.pip_metal_pct, t);
    let max_size = if rule.pip_max_size > 0.0 {
        rule.pip_max_size
    } else {
        pad.pitch.map_or(f64::INFINITY, |p| (p - pad.min_bridge.max(0.1)).max(0.2))
    };
    let min_size = w_eff.min(h_eff);
    let mut out = match rule.pip_shape.as_str() {
        "rect" => {
            let side = area.sqrt().clamp(min_size.min(max_size), max_size);
            vec![rounded_slice(cx, cy, side, side, rule.corner_radius, "pin_in_paste")]
        }
        "donut" => {
            let inner = drill + 0.1;
            let outer = (4.0 * area / PI + inner * inner).sqrt().clamp(min_size.min(max_size), max_size);
            if outer <= inner * 1.05 {
                vec![circle_slice(cx, cy, outer, "pin_in_paste")]
            } else {
                donut_segments(cx, cy, inner, outer, rule.bridge_width.max(0.1))
            }
        }
        _ => {
            let d = (4.0 * area / PI).sqrt().clamp(min_size.min(max_size), max_size);
            vec![circle_slice(cx, cy, d, "pin_in_paste")]
        }
    };
    for s in out.iter_mut() {
        s.kind = Some("pin_in_paste".into());
        s.manual = None;
    }
    out
}

/// Regel "Package-Gruppe" (angelehnt an PCB Investigator "Handle Stencil Creation by Package Group"):
/// - BGA: runde/quadratische Pads ≤ BGA-Schwelle → 1:1-Form minus Undersize
/// - Große Flächen (beide Seiten ≥ 2 × Mindestfeldgröße) → Array-Felder mit Feldabstand und Flächennutzung
/// - 2-Pin-Chips mit Pad > Chip-Schwelle → HouseBuilder; Pads kleiner als "Invert House" → Inverted House
///   mit zusätzlicher Mini-Chip-Reduktion
/// - sonst Rechteck mit Undersize und Eckradius
fn package_group_geometry(pad: &PadCtx, rule: &PackageRule, cx: f64, cy: f64, w_eff: f64, h_eff: f64) -> Vec<Slice> {
    let (pw, ph) = (pad.w, pad.h);
    let small = pw.min(ph);
    let large = pw.max(ph);
    let squareish = (pw - ph).abs() <= 0.1 * large;

    // BGA-/CSP-Kugeln: kleine (fast) quadratische Pads werden wie im PCB Investigator rund geöffnet
    if pad.round || (squareish && large <= rule.bga_threshold_square && pad.pin_count != 2) {
        return vec![circle_slice(cx, cy, w_eff.min(h_eff), "package_group_bga")];
    }
    if rule.min_array_field > 0.0 && small >= 2.0 * rule.min_array_field && pad.pin_count != 2 {
        return array_fields(cx, cy, w_eff, h_eff, rule.min_array_field, rule.distance_array_fields, rule.utilization_array_pct / 100.0, rule.corner_radius);
    }
    // 2-Pin-Chips: bis zur Chip-Schwelle abgerundetes Rechteck (Abgleich mit PCB-I-Referenzjob:
    // 0201…0603, SOD923 werden nicht als Haus geöffnet); darüber Haus bzw. Inverted House
    if pad.pin_count == 2 && large > rule.chip_threshold_houses {
        let mut r = rule.clone();
        let (mut w, mut h) = (w_eff, h_eff);
        if small < rule.invert_house_below {
            r.rule_type = "inverted_house_builder".into();
            w = (w - 2.0 * rule.mini_chip_reduction).max(0.04);
            h = (h - 2.0 * rule.mini_chip_reduction).max(0.04);
        } else {
            r.rule_type = "house_builder".into();
        }
        if rule.utilization_house_pct > 0.0 {
            let s = (rule.utilization_house_pct / 100.0).clamp(0.05, 1.0).sqrt();
            w *= s;
            h *= s;
        }
        r.undersize_w = 0.0;
        r.undersize_h = 0.0;
        r.offset_x = 0.0;
        r.offset_y = 0.0;
        r.offset_move_away = 0.0;
        r.toe_heel_shift = 0.0;
        let sub = PadCtx { w, h, cx, cy, mask: None, ..*pad };
        if let Some(mut g) = generate_rule_aperture_geometry(&sub, &r) {
            for s in g.iter_mut() {
                s.kind = Some(format!("package_group_{}", r.rule_type));
            }
            return g;
        }
    }
    vec![rounded_slice(cx, cy, w_eff, h_eff, rule.corner_radius, "package_group_rect")]
}

/// Erlaubte Intervalle entlang einer Richtung, die nach Abzug paralleler Stege übrig bleiben.
fn free_bands(count: usize, span: f64, web: f64) -> Vec<(f64, f64)> {
    let step = span / (count as f64 + 1.0);
    let mut bands = Vec::with_capacity(count + 1);
    let mut lo = f64::NEG_INFINITY;
    for i in 1..=count {
        let off = -span / 2.0 + i as f64 * step;
        bands.push((lo, off - web / 2.0));
        lo = off + web / 2.0;
    }
    bands.push((lo, f64::INFINITY));
    bands
}

/// 45°-Diagonalstege (Squeegee-Scooping-Schutz): Pad-Rechteck minus diagonale Stegstreifen.
/// Da alle Restflächen Schnitte konvexer Bänder sind, genügt exaktes Halbebenen-Clipping.
fn diagonal_slices(cx: f64, cy: f64, w: f64, h: f64, rule: &PackageRule) -> Vec<Slice> {
    let web = rule.bridge_width.max(0.08);
    let span = w.hypot(h) * 1.5;
    let s2 = std::f64::consts::FRAC_1_SQRT_2;

    let nx = rule.bridge_count_x.max(1) as usize;
    // Streifen um +45° gedreht: Koordinate quer zum Streifen v = (-dx + dy)/√2
    let bands_a = free_bands(nx, span, web);
    // Optional zweite Schar um -45°: v = (dx + dy)/√2
    let bands_b = if rule.bridge_count_y > 0 {
        free_bands(rule.bridge_count_y as usize, span, web)
    } else {
        vec![(f64::NEG_INFINITY, f64::INFINITY)]
    };

    let base = rect_pts(cx, cy, w, h);
    let mut out = Vec::new();
    for &(a_lo, a_hi) in &bands_a {
        for &(b_lo, b_hi) in &bands_b {
            let mut poly = base.clone();
            // v_a >= a_lo  <=>  (-x + y)/√2 >= a_lo + (-cx + cy)/√2
            let ca = (-cx + cy) * s2;
            let cb = (cx + cy) * s2;
            if a_lo.is_finite() {
                poly = clip_halfplane(&poly, -s2, s2, a_lo + ca);
            }
            if a_hi.is_finite() {
                poly = clip_halfplane(&poly, s2, -s2, -(a_hi + ca));
            }
            if b_lo.is_finite() {
                poly = clip_halfplane(&poly, s2, s2, b_lo + cb);
            }
            if b_hi.is_finite() {
                poly = clip_halfplane(&poly, -s2, -s2, -(b_hi + cb));
            }
            if poly.len() < 3 {
                continue;
            }
            let area = polygon_area(&poly);
            if area < 0.005 {
                continue;
            }
            let (x0, y0, x1, y1) = bounds(&poly);
            let (pcx, pcy) = centroid(&poly);
            let mut s = poly_slice(pcx, pcy, x1 - x0, y1 - y0, &poly, "array_slicing_45");
            s.area = Some(r5(area));
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_slicing_removes_web_area() {
        let rule = PackageRule { rule_type: "array_slicing".into(), rotate_bridges_45: true, bridge_count_x: 2, bridge_count_y: 2, bridge_width: 0.3, undersize_w: 0.0, undersize_h: 0.0, ..Default::default() };
        let s = generate_rule_aperture_geometry(&PadCtx::new(4.0, 4.0, 0.0, 0.0, "center"), &rule).unwrap();
        assert!(s.len() >= 4);
        let total: f64 = s.iter().map(|x| x.area.unwrap()).sum();
        assert!(total < 16.0 && total > 8.0, "Restfläche {total}");
    }

    #[test]
    fn orthogonal_slicing_grid() {
        let rule = PackageRule { rule_type: "array_slicing".into(), bridge_count_x: 1, bridge_count_y: 1, bridge_width: 0.2, undersize_w: 0.0, undersize_h: 0.0, ..Default::default() };
        let s = generate_rule_aperture_geometry(&PadCtx::new(2.2, 2.2, 0.0, 0.0, "center"), &rule).unwrap();
        assert_eq!(s.len(), 4);
        assert!((s[0].w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rotated_pad_keeps_its_shape() {
        // 0,8 × 0,4 mm Pad um 45° gedreht: Öffnung folgt der Kontur statt der Bounding Box (≈0,85 × 0,85)
        let pts = crate::engine::geometry::rotate_pts(&rect_pts(0.0, 0.0, 0.8, 0.4), 0.0, 0.0, 45.0);
        let rule = PackageRule { rule_type: "package_group".into(), undersize_w: 0.025, undersize_h: 0.025, ..Default::default() };
        let s = shape_true_opening(&pts, &rule).unwrap();
        let area = polygon_area(s[0].polygon_pts().unwrap());
        assert!((area - 0.75 * 0.35).abs() < 1e-4, "{area}");
        assert!((crate::engine::geometry::min_width(s[0].polygon_pts().unwrap()) - 0.35).abs() < 1e-4);
        // Achsparalleles abgerundetes Rechteck bleibt bei der normalen Regel
        assert!(shape_true_opening(&rounded_rect_pts(0.0, 0.0, 0.8, 0.4, 0.05), &rule).is_none());
    }

    #[test]
    fn take_existing_and_ignore() {
        let mut rule = PackageRule::default();
        rule.rule_type = "take_existing".into();
        assert!(generate_rule_aperture_geometry(&PadCtx::new(1.0, 1.0, 0.0, 0.0, "right"), &rule).is_none());
        rule.rule_type = "ignore".into();
        assert!(generate_rule_aperture_geometry(&PadCtx::new(1.0, 1.0, 0.0, 0.0, "right"), &rule).unwrap().is_empty());
        // Rechteckige Kundenpaste wird abgerundet, Radius 0 lässt sie eckig
        rule.rule_type = "take_existing".into();
        let s = take_existing_rounded(&rule, true, 0.0, 0.0, 1.0, 0.5).unwrap();
        assert_eq!(s[0].radius, Some(0.075));
        assert!(take_existing_rounded(&rule, false, 0.0, 0.0, 1.0, 0.5).is_none());
        rule.corner_radius = 0.0;
        assert!(take_existing_rounded(&rule, true, 0.0, 0.0, 1.0, 0.5).is_none());
    }

    #[test]
    fn round_pads_stay_round() {
        let rule = PackageRule { rule_type: "house_builder".into(), undersize_w: 0.05, undersize_h: 0.05, ..Default::default() };
        let s = generate_rule_aperture_geometry(&PadCtx { round: true, ..PadCtx::new(1.5, 1.5, 2.0, 3.0, "right") }, &rule).unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].is_circle());
        assert!((s[0].w - 1.4).abs() < 1e-9);
    }

    #[test]
    fn elongated_respects_pitch() {
        // 0804-Array: Pads 0.35 × 0.65 mm im 0.5 mm Pitch, Pad links vom Bauteilzentrum (facing right).
        // Pastenabstand = Öffnungsbreite → 0.25 mm; Länge 0.65 × (95 % + 70 % × 28.6 %) ≈ 0.747 mm;
        // 0.025 mm nach außen (links) versetzt.
        let rule = PackageRule { rule_type: "elongated_pitch".into(), ..Default::default() };
        let ctx = PadCtx { pitch: Some(0.5), min_bridge: 0.1, ..PadCtx::new(0.35, 0.65, 0.0, 0.0, "right") };
        let s = generate_rule_aperture_geometry(&ctx, &rule).unwrap();
        assert!((s[0].w - 0.25).abs() < 1e-6, "Breite {}", s[0].w);
        assert!((s[0].h - 0.7475).abs() < 1e-3, "Länge {}", s[0].h);
        assert!((s[0].cx + 0.025).abs() < 1e-6, "Versatz {}", s[0].cx);
        // Abstand doppelt so groß wie die Öffnung → Breite = Pitch / 3
        let wide_gap = PackageRule { gap_ratio: 2.0, outward_shift: 0.0, length_comp_pct: 0.0, ..rule.clone() };
        let s = generate_rule_aperture_geometry(&ctx, &wide_gap).unwrap();
        assert!((s[0].w - 0.5 / 3.0).abs() < 1e-3);
        assert!((s[0].h - 0.6175).abs() < 1e-6 && s[0].cx.abs() < 1e-9);
        // Großzügiger Pitch: nur die prozentuale Reduktion
        let ctx = PadCtx { pitch: Some(3.0), ..PadCtx::new(3.0, 1.0, 0.0, 0.0, "left") };
        let s = generate_rule_aperture_geometry(&ctx, &rule).unwrap();
        assert!((s[0].h - 0.8).abs() < 1e-9);
    }

    #[test]
    fn move_away_axis() {
        // Zweireihiger Stecker (FTSH): Pads 1.5 × 0.4 mm, Eckpad oben links. Zentrum liegt eher
        // unterhalb (facing "down"), auseinanderschieben soll aber nur in X.
        let rule = PackageRule { rule_type: "simple_rect".into(), offset_move_away: 0.3, undersize_w: 0.0, undersize_h: 0.0, ..Default::default() };
        let pad = PadCtx { rel: Some((-1.0, 2.5)), ..PadCtx::new(1.5, 0.4, -1.0, 2.5, "down") };
        let s = generate_rule_aperture_geometry(&pad, &rule).unwrap();
        assert!((s[0].cx + 1.3).abs() < 1e-9 && (s[0].cy - 2.5).abs() < 1e-9, "Längsachse: {:?}", (s[0].cx, s[0].cy));
        // "auto": wie bisher Richtung Bauteilzentrum (hier nach oben)
        let auto = PackageRule { move_axis: "auto".into(), ..rule.clone() };
        let s = generate_rule_aperture_geometry(&pad, &auto).unwrap();
        assert!((s[0].cy - 2.8).abs() < 1e-9 && (s[0].cx + 1.0).abs() < 1e-9);
        // "x" im Package-Rahmen bei um 90° gedrehtem Bauteil: Package-X liegt auf dem Board in Y
        let x = PackageRule { move_axis: "x".into(), ..rule.clone() };
        let rel = pkg_to_frame((-1.0, 2.5), 90.0, false);
        let rot = PadCtx { rel: Some(rel), rot: 90.0, ..PadCtx::new(0.4, 1.5, rel.0, rel.1, "right") };
        let s = generate_rule_aperture_geometry(&rot, &x).unwrap();
        let moved = (s[0].cx - rel.0, s[0].cy - rel.1);
        let back = frame_to_pkg(moved, 90.0, false);
        assert!((back.0 + 0.3).abs() < 1e-9 && back.1.abs() < 1e-9, "{back:?}");
        // Offset X/Y dreht mit dem Bauteil
        let off = PackageRule { offset_x: 0.1, offset_move_away: 0.0, ..rule.clone() };
        let s = generate_rule_aperture_geometry(&PadCtx { rot: 90.0, ..PadCtx::new(0.4, 1.5, 0.0, 0.0, "right") }, &off).unwrap();
        assert!(s[0].cx.abs() < 1e-9 && (s[0].cy + 0.1).abs() < 1e-9, "{:?}", (s[0].cx, s[0].cy));
    }

    #[test]
    fn house_is_rounded() {
        let rule = PackageRule { rule_type: "house_builder".into(), corner_radius: 0.075, undersize_w: 0.0, undersize_h: 0.0, ..Default::default() };
        let s = generate_rule_aperture_geometry(&PadCtx::new(0.6, 0.8, 0.0, 0.0, "right"), &rule).unwrap();
        let pts = s[0].pts.as_ref().unwrap();
        assert!(pts.len() > 20, "Ecken abgerundet: {} Punkte", pts.len());
        // Spitze abgerundet: kein Punkt erreicht mehr x = +0.3
        assert!(pts.iter().all(|p| p.0 < 0.299));
        assert!(pts.iter().all(|p| p.0 >= -0.3 - 1e-9 && p.1.abs() <= 0.4 + 1e-9));
    }

    #[test]
    fn package_group_picks_shape_per_pad() {
        let rule = PackageRule { rule_type: "package_group".into(), ..Default::default() };
        // 0603-Chip (≤ Chip-Schwelle) → abgerundetes Rechteck vom Kupfer minus Undersize (wie PCB-I)
        let chip = PadCtx { pin_count: 2, copper: Some((0.56, 0.89, false)), ..PadCtx::new(0.5, 0.8, 0.0, 0.0, "right") };
        let s = generate_rule_aperture_geometry(&chip, &rule).unwrap();
        assert_eq!(s[0].kind.as_deref(), Some("package_group_rect"));
        assert!((s[0].w - 0.51).abs() < 1e-6 && (s[0].h - 0.84).abs() < 1e-6, "{:?}", (s[0].w, s[0].h));
        // großer 2-Pin-Leistungschip (> Chip-Schwelle) → HouseBuilder
        let big = PadCtx { pin_count: 2, ..PadCtx::new(1.2, 2.8, 0.0, 0.0, "right") };
        let s = generate_rule_aperture_geometry(&big, &rule).unwrap();
        assert_eq!(s[0].kind.as_deref(), Some("package_group_house_builder"));
        // BGA-Pad (fast quadratisch, ≤ BGA-Schwelle) → rund
        let ball = PadCtx { pin_count: 196, copper: Some((0.3, 0.3, true)), ..PadCtx::new(0.3, 0.3, 0.0, 0.0, "center") };
        let s = generate_rule_aperture_geometry(&ball, &rule).unwrap();
        assert!(s[0].is_circle() && (s[0].w - 0.25).abs() < 1e-6);
        // Thermal-Pad 4×4 mm im QFN → Array-Felder mit ≈ 40 % Fläche
        let th = PadCtx { pin_count: 49, ..PadCtx::new(4.0, 4.0, 0.0, 0.0, "center") };
        let s = generate_rule_aperture_geometry(&th, &rule).unwrap();
        assert!(s.len() >= 4);
        let area: f64 = s.iter().map(|x| x.w * x.h).sum();
        assert!(area > 0.25 * 15.8 && area < 0.6 * 15.8, "Fläche {area}");
        // BGA-Ball rund bleibt rund
        let bga = PadCtx { round: true, pin_count: 201, ..PadCtx::new(0.3, 0.3, 0.0, 0.0, "center") };
        assert!(generate_rule_aperture_geometry(&bga, &rule).unwrap()[0].is_circle());
    }

    #[test]
    fn simple_rect_rounded_and_fixed_size() {
        let rule = PackageRule { size_w: 0.8, size_h: 0.5, corner_radius: 0.075, ..Default::default() };
        let s = generate_rule_aperture_geometry(&PadCtx::new(1.2, 1.3, 0.0, 0.0, "right"), &rule).unwrap();
        assert!((s[0].w - 0.8).abs() < 1e-9 && (s[0].h - 0.5).abs() < 1e-9);
        assert!(s[0].pts.as_ref().unwrap().len() > 20, "echte Bogenecken erwartet");
    }

    #[test]
    fn pin_in_paste_size_from_drill() {
        let rule = PackageRule { rule_type: "pin_in_paste".into(), ..Default::default() };
        let ctx = PadCtx { round: true, drill: Some(1.0), thickness: 0.12, pin_count: 20, ..PadCtx::new(1.8, 1.8, 0.0, 0.0, "center") };
        let s = generate_rule_aperture_geometry(&ctx, &rule).unwrap();
        let need = pip_required_area(1.0, 0.75, 1.6, 50.0, 0.12);
        let d = (4.0 * need / std::f64::consts::PI).sqrt();
        assert!((s[0].w - r4(d)).abs() < 1e-3, "{} vs {d}", s[0].w);
        // durch Pitch begrenzt
        let ctx = PadCtx { pitch: Some(2.54), min_bridge: 0.15, ..ctx };
        let s = generate_rule_aperture_geometry(&ctx, &rule).unwrap();
        assert!(s[0].w <= 2.39 + 1e-9);
    }

    #[test]
    fn pin_override_keeps_base_values() {
        let mut base = PackageRule::default();
        base.offset_move_away = 0.05;
        let mut m = Map::new();
        m.insert("rule_type".into(), Value::String("inherit".into()));
        m.insert("undersize_w".into(), serde_json::json!(0.1));
        let r = resolve_pin_rule(&base, &m);
        assert_eq!(r.rule_type, "simple_rect");
        assert_eq!(r.undersize_w, 0.1);
        assert_eq!(r.offset_move_away, 0.05);
    }
}
