//! Regelbasierte Apertur-Geometrie (Simple Rectangle, HouseBuilder, Inverted House, Chamfer,
//! Länglich & Schmal, Array Slicing orthogonal und 45°-diagonal).

use super::geometry::{bounds, centroid, circle_pts, clip_halfplane, polygon_area, rect_pts, rounded_rect_pts};
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

/// Erzeugt die Schablonenöffnung(en) für ein Pad nach einer Package-Regel.
/// `None` = Original übernehmen (take_existing), leere Liste = Pad ignorieren.
#[allow(clippy::too_many_arguments)]
pub fn generate_rule_aperture_geometry(
    pad_w: f64,
    pad_h: f64,
    pad_cx: f64,
    pad_cy: f64,
    rule: &PackageRule,
    facing_dir: &str,
    mask_w: Option<f64>,
    mask_h: Option<f64>,
    round: bool,
) -> Option<Vec<Slice>> {
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

    let mut cx = pad_cx + rule.offset_x;
    let mut cy = pad_cy + rule.offset_y;

    if rule.offset_move_away != 0.0 {
        match facing_dir {
            "right" => cx -= rule.offset_move_away,
            "left" => cx += rule.offset_move_away,
            "up" => cy -= rule.offset_move_away,
            "down" => cy += rule.offset_move_away,
            _ => {}
        }
    }

    // Toe-/Heel-Shift (>0 nach außen / Toe, <0 nach innen / Heel)
    if rule.toe_heel_shift != 0.0 {
        let s = rule.toe_heel_shift;
        match facing_dir {
            "right" => cx += s,
            "left" => cx -= s,
            "up" => cy += s,
            "down" => cy -= s,
            _ => {}
        }
    }

    // Runde Pads (Stecker, THT, BGA) bleiben rund: Undersize/Offsets wirken auf den Durchmesser.
    // Rechteck-spezifische Formen (House, Chamfer, Länglich) sind für Kreise nicht sinnvoll.
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
            let r = rule.corner_radius.min(w_eff / 2.2).min(h_eff / 2.2);
            if r > 0.01 {
                let mut s = poly_slice(cx, cy, w_eff, h_eff, &rounded_rect_pts(cx, cy, w_eff, h_eff, r), "simple_rect");
                s.radius = Some(r4(r));
                Some(vec![s])
            } else {
                Some(vec![rect_slice(cx, cy, w_eff, h_eff, Some("simple_rect"))])
            }
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
            Some(vec![poly_slice(cx, cy, w_eff, h_eff, &pts, "house_builder")])
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
            Some(vec![poly_slice(cx, cy, w_eff, h_eff, &pts, "inverted_house_builder")])
        }
        "elongated_pitch" => {
            let (new_w, new_h) = if pad_w <= pad_h {
                (pad_w * (1.0 - rule.pitch_reduction_pct / 100.0), pad_h * (rule.length_usage_pct / 100.0))
            } else {
                (pad_w * (rule.length_usage_pct / 100.0), pad_h * (1.0 - rule.pitch_reduction_pct / 100.0))
            };
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
            let web = rule.bridge_width.max(0.08);
            let sub_w = (w_eff - (gx as f64 - 1.0) * web) / gx as f64;
            let sub_h = (h_eff - (gy as f64 - 1.0) * web) / gy as f64;
            if sub_w <= 0.0 || sub_h <= 0.0 {
                return Some(vec![rect_slice(cx, cy, w_eff, h_eff, None)]);
            }
            let start_x = cx - w_eff / 2.0 + sub_w / 2.0;
            let start_y = cy - h_eff / 2.0 + sub_h / 2.0;
            let mut out = Vec::with_capacity(gx * gy);
            for ix in 0..gx {
                for iy in 0..gy {
                    out.push(rect_slice(start_x + ix as f64 * (sub_w + web), start_y + iy as f64 * (sub_h + web), sub_w, sub_h, Some("array_slicing")));
                }
            }
            Some(out)
        }
        _ => Some(vec![rect_slice(cx, cy, w_eff, h_eff, None)]),
    }
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
        let s = generate_rule_aperture_geometry(4.0, 4.0, 0.0, 0.0, &rule, "center", None, None, false).unwrap();
        assert!(s.len() >= 4);
        let total: f64 = s.iter().map(|x| x.area.unwrap()).sum();
        assert!(total < 16.0 && total > 8.0, "Restfläche {total}");
    }

    #[test]
    fn orthogonal_slicing_grid() {
        let rule = PackageRule { rule_type: "array_slicing".into(), bridge_count_x: 1, bridge_count_y: 1, bridge_width: 0.2, undersize_w: 0.0, undersize_h: 0.0, ..Default::default() };
        let s = generate_rule_aperture_geometry(2.2, 2.2, 0.0, 0.0, &rule, "center", None, None, false).unwrap();
        assert_eq!(s.len(), 4);
        assert!((s[0].w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn take_existing_and_ignore() {
        let mut rule = PackageRule::default();
        rule.rule_type = "take_existing".into();
        assert!(generate_rule_aperture_geometry(1.0, 1.0, 0.0, 0.0, &rule, "right", None, None, false).is_none());
        rule.rule_type = "ignore".into();
        assert!(generate_rule_aperture_geometry(1.0, 1.0, 0.0, 0.0, &rule, "right", None, None, false).unwrap().is_empty());
    }

    #[test]
    fn round_pads_stay_round() {
        let rule = PackageRule { rule_type: "house_builder".into(), undersize_w: 0.05, undersize_h: 0.05, ..Default::default() };
        let s = generate_rule_aperture_geometry(1.5, 1.5, 2.0, 3.0, &rule, "right", None, None, true).unwrap();
        assert_eq!(s.len(), 1);
        assert!(s[0].is_circle());
        assert!((s[0].w - 1.4).abs() < 1e-9);
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
