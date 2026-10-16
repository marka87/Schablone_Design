use crate::model::Point;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChamferMode {
    All,
    Pin1,
    Toe,
    Heel,
}

/// Berechnet den Flächeninhalt eines Polygons nach der Gaußschen Trapezformel (Shoelace-Formel).
pub fn polygon_area(pts: &[Point]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        area += pts[i].x * pts[j].y;
        area -= pts[j].x * pts[i].y;
    }
    (area / 2.0).abs()
}

/// 1. Rectangle mit Breiten- und Höhenreduktion (Undersize).
pub fn calc_rectangle(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    undersize_w: f64,
    undersize_h: f64,
) -> (Vec<Point>, f64) {
    let w_eff = (w - 2.0 * undersize_w).max(0.01);
    let h_eff = (h - 2.0 * undersize_h).max(0.01);
    let hw = w_eff / 2.0;
    let hh = h_eff / 2.0;

    let pts = vec![
        Point::new(cx - hw, cy - hh),
        Point::new(cx + hw, cy - hh),
        Point::new(cx + hw, cy + hh),
        Point::new(cx - hw, cy + hh),
    ];
    let area = w_eff * h_eff;
    (pts, area)
}

/// 2. Eckverrundung (Corner Radius) als 8-Punkt oder N-Segment Polygon.
pub fn calc_corner_radius(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    radius: f64,
    segments_per_corner: usize,
) -> (Vec<Point>, f64) {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    if r <= 1e-4 {
        return calc_rectangle(cx, cy, w, h, 0.0, 0.0);
    }

    let hw = w / 2.0;
    let hh = h / 2.0;
    let segs = segments_per_corner.max(1);

    let centers = [
        (cx + hw - r, cy - hh + r, 270.0, 360.0), // unten rechts
        (cx + hw - r, cy + hh - r, 0.0, 90.0),    // oben rechts
        (cx - hw + r, cy + hh - r, 90.0, 180.0),  // oben links
        (cx - hw + r, cy - hh + r, 180.0, 270.0), // unten links
    ];

    let mut pts = Vec::with_capacity(centers.len() * (segs + 1));
    for (ccx, ccy, a_start, a_end) in centers {
        for s in 0..=segs {
            let frac = s as f64 / segs as f64;
            let angle = (a_start + frac * (a_end - a_start)).to_radians();
            pts.push(Point::new(ccx + r * angle.cos(), ccy + r * angle.sin()));
        }
    }

    let area = polygon_area(&pts);
    (pts, area)
}

/// 3. Chamfered Rectangle (45°-Eckanfasung).
pub fn calc_chamfer(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    chamfer_size: f64,
    mode: ChamferMode,
    facing_dir: &str,
) -> (Vec<Point>, f64) {
    let c = chamfer_size.min(w / 2.0).min(h / 2.0).max(0.0);
    if c <= 1e-4 {
        return calc_rectangle(cx, cy, w, h, 0.0, 0.0);
    }

    let x0 = cx - w / 2.0;
    let x1 = cx + w / 2.0;
    let y0 = cy - h / 2.0;
    let y1 = cy + h / 2.0;

    let (c_bl, c_br, c_tr, c_tl) = match mode {
        ChamferMode::All => (c, c, c, c),
        ChamferMode::Pin1 => (0.0, 0.0, 0.0, c), // top-left ist Pin 1
        ChamferMode::Toe => match facing_dir.to_lowercase().as_str() {
            "right" => (0.0, c, c, 0.0),
            "left" => (c, 0.0, 0.0, c),
            "up" => (0.0, 0.0, c, c),
            _ => (c, c, 0.0, 0.0),
        },
        ChamferMode::Heel => match facing_dir.to_lowercase().as_str() {
            "right" => (c, 0.0, 0.0, c),
            "left" => (0.0, c, c, 0.0),
            "up" => (c, c, 0.0, 0.0),
            _ => (0.0, 0.0, c, c),
        },
    };

    let mut pts = Vec::with_capacity(8);
    // Unten (y0)
    pts.push(Point::new(x0 + c_bl, y0));
    pts.push(Point::new(x1 - c_br, y0));
    // Rechts (x1)
    pts.push(Point::new(x1, y0 + c_br));
    pts.push(Point::new(x1, y1 - c_tr));
    // Oben (y1)
    pts.push(Point::new(x1 - c_tr, y1));
    pts.push(Point::new(x0 + c_tl, y1));
    // Links (x0)
    pts.push(Point::new(x0, y1 - c_tl));
    pts.push(Point::new(x0, y0 + c_bl));

    // Duplikate entfernen falls Ecke kein Chamfer hat
    pts.dedup_by(|a, b| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);

    let area = polygon_area(&pts);
    (pts, area)
}

/// 4. HouseBuilder (Giebel-förmige Apertur gegen Tombstoning & Lötbrücken).
pub fn calc_house_builder(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    cut_pct: f64,
    facing_dir: &str,
) -> (Vec<Point>, f64) {
    let hw = w / 2.0;
    let hh = h / 2.0;
    let cut_w = hw * (cut_pct / 100.0).clamp(0.05, 0.85);
    let cut_h = hh * (cut_pct / 100.0).clamp(0.05, 0.85);

    let pts = match facing_dir.to_lowercase().as_str() {
        "right" => vec![
            Point::new(cx - hw, cy - hh),
            Point::new(cx + hw - cut_w, cy - hh),
            Point::new(cx + hw, cy),
            Point::new(cx + hw - cut_w, cy + hh),
            Point::new(cx - hw, cy + hh),
        ],
        "left" => vec![
            Point::new(cx + hw, cy - hh),
            Point::new(cx - hw + cut_w, cy - hh),
            Point::new(cx - hw, cy),
            Point::new(cx - hw + cut_w, cy + hh),
            Point::new(cx + hw, cy + hh),
        ],
        "up" => vec![
            Point::new(cx - hw, cy - hh),
            Point::new(cx - hw, cy + hh - cut_h),
            Point::new(cx, cy + hh),
            Point::new(cx + hw, cy + hh - cut_h),
            Point::new(cx + hw, cy - hh),
        ],
        _ => vec![
            // down (default)
            Point::new(cx - hw, cy + hh),
            Point::new(cx - hw, cy - hh + cut_h),
            Point::new(cx, cy - hh),
            Point::new(cx + hw, cy - hh + cut_h),
            Point::new(cx + hw, cy + hh),
        ],
    };

    let area = polygon_area(&pts);
    (pts, area)
}

/// 5. InvertedHouseBuilder (Konvexe Häuschen-Form).
pub fn calc_inverted_house_builder(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    cut_pct: f64,
    facing_dir: &str,
) -> (Vec<Point>, f64) {
    let hw = w / 2.0;
    let hh = h / 2.0;
    let cut_w = hw * (cut_pct / 100.0).clamp(0.05, 0.85);
    let cut_h = hh * (cut_pct / 100.0).clamp(0.05, 0.85);

    let pts = match facing_dir.to_lowercase().as_str() {
        "right" => vec![
            Point::new(cx + hw, cy - hh),
            Point::new(cx - hw + cut_w, cy - hh),
            Point::new(cx - hw, cy),
            Point::new(cx - hw + cut_w, cy + hh),
            Point::new(cx + hw, cy + hh),
        ],
        "left" => vec![
            Point::new(cx - hw, cy - hh),
            Point::new(cx + hw - cut_w, cy - hh),
            Point::new(cx + hw, cy),
            Point::new(cx + hw - cut_w, cy + hh),
            Point::new(cx - hw, cy + hh),
        ],
        "up" => vec![
            Point::new(cx - hw, cy + hh),
            Point::new(cx + hw, cy + hh),
            Point::new(cx + hw, cy - hh + cut_h),
            Point::new(cx, cy - hh),
            Point::new(cx - hw, cy - hh + cut_h),
        ],
        _ => vec![
            // down (default)
            Point::new(cx - hw, cy - hh),
            Point::new(cx - hw, cy + hh - cut_h),
            Point::new(cx, cy + hh),
            Point::new(cx + hw, cy + hh - cut_h),
            Point::new(cx + hw, cy - hh),
        ],
    };

    let area = polygon_area(&pts);
    (pts, area)
}

/// 6. Array Slicing (N x M Fensterung mit orthogonalen oder 45°-diagonalen Stegen).
pub fn calc_array_slicing(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    nx: usize,
    my: usize,
    bridge_w: f64,
    rotate_45: bool,
) -> Vec<(Vec<Point>, f64)> {
    let grid_x = nx.max(1);
    let grid_y = my.max(1);
    let bridge = bridge_w.max(0.05);

    if !rotate_45 {
        // Orthogonales Slicing
        let sub_w = (w - (grid_x - 1) as f64 * bridge) / grid_x as f64;
        let sub_h = (h - (grid_y - 1) as f64 * bridge) / grid_y as f64;

        if sub_w <= 0.05 || sub_h <= 0.05 {
            return vec![calc_rectangle(cx, cy, w, h, 0.0, 0.0)];
        }

        let start_x = cx - w / 2.0 + sub_w / 2.0;
        let start_y = cy - h / 2.0 + sub_h / 2.0;
        let mut slices = Vec::with_capacity(grid_x * grid_y);

        for ix in 0..grid_x {
            for iy in 0..grid_y {
                let scx = start_x + ix as f64 * (sub_w + bridge);
                let scy = start_y + iy as f64 * (sub_h + bridge);
                slices.push(calc_rectangle(scx, scy, sub_w, sub_h, 0.0, 0.0));
            }
        }
        slices
    } else {
        // 45°-Diagonales Slicing (Squeegee Scooping Prevention) via Sutherland-Hodgman Polygon Clipping
        let total_cuts = (grid_x + grid_y).max(2);
        let diag_span = (w * w + h * h).sqrt();
        let step = diag_span / (total_cuts as f64);

        let base_poly = vec![
            Point::new(cx - w / 2.0, cy - h / 2.0),
            Point::new(cx + w / 2.0, cy - h / 2.0),
            Point::new(cx + w / 2.0, cy + h / 2.0),
            Point::new(cx - w / 2.0, cy + h / 2.0),
        ];

        let mut slices = Vec::new();
        let half_bridge = bridge / (2.0_f64.sqrt());

        for i in 0..total_cuts {
            let offset_start = -diag_span / 2.0 + i as f64 * step + half_bridge;
            let offset_end = -diag_span / 2.0 + (i + 1) as f64 * step - half_bridge;

            if offset_end <= offset_start {
                continue;
            }

            // Halb-Ebenen: (x - cx) - (y - cy) >= offset_start  UND  (x - cx) - (y - cy) <= offset_end
            // a*x + b*y + c >= 0 mit a=1, b=-1, c = -(cx - cy) - offset_start
            let c1 = -(cx - cy) - offset_start;
            let p1 = clip_polygon_halfplane(&base_poly, 1.0, -1.0, c1);

            // -x + y + (cx - cy) + offset_end >= 0
            let c2 = (cx - cy) + offset_end;
            let final_poly = clip_polygon_halfplane(&p1, -1.0, 1.0, c2);

            let area = polygon_area(&final_poly);
            if area > 0.02 && final_poly.len() >= 3 {
                slices.push((final_poly, area));
            }
        }

        if slices.is_empty() {
            vec![calc_rectangle(cx, cy, w, h, 0.0, 0.0)]
        } else {
            slices
        }
    }
}

fn line_intersection(p1: &Point, p2: &Point, a: f64, b: f64, c: f64) -> Option<Point> {
    let dx = p2.x - p1.x;
    let dy = p2.y - p1.y;
    let denom = a * dx + b * dy;
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = -(a * p1.x + b * p1.y + c) / denom;
    Some(Point::new(p1.x + t * dx, p1.y + t * dy))
}

/// Sutherland-Hodgman Polygon-Clipping an der Halb-Ebene a*x + b*y + c >= 0
fn clip_polygon_halfplane(poly: &[Point], a: f64, b: f64, c: f64) -> Vec<Point> {
    if poly.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let is_inside = |p: &Point| a * p.x + b * p.y + c >= -1e-7;

    for i in 0..poly.len() {
        let curr = &poly[i];
        let next = &poly[(i + 1) % poly.len()];

        let in_curr = is_inside(curr);
        let in_next = is_inside(next);

        if in_curr && in_next {
            out.push(*next);
        } else if in_curr && !in_next {
            if let Some(inter) = line_intersection(curr, next, a, b, c) {
                out.push(inter);
            }
        } else if !in_curr && in_next {
            if let Some(inter) = line_intersection(curr, next, a, b, c) {
                out.push(inter);
            }
            out.push(*next);
        }
    }
    out
}

/// 7. Länglich & Schmal (Pitch-Schutz: Schmälert Pads quer zur Längsachse gegen Lötbrücken)
pub fn calc_elongated_pitch(
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    pitch_reduction_pct: f64,
    radius: f64,
) -> (Vec<Point>, f64) {
    let red = (pitch_reduction_pct / 100.0).clamp(0.0, 0.50);
    let (new_w, new_h) = if w <= h {
        (w * (1.0 - red), h)
    } else {
        (w, h * (1.0 - red))
    };
    if radius > 0.01 {
        calc_corner_radius(cx, cy, new_w, new_h, radius, 2)
    } else {
        calc_rectangle(cx, cy, new_w, new_h, 0.0, 0.0)
    }
}

/// Rotiert Polygon-Punkte um einen Drehpunkt (cx, cy) um den Winkel rot_deg.
pub fn rotate_polygon(pts: &[Point], cx: f64, cy: f64, rot_deg: f64) -> Vec<Point> {
    if rot_deg.abs() < 1e-4 {
        return pts.to_vec();
    }
    let rad = rot_deg.to_radians();
    let cos_a = rad.cos();
    let sin_a = rad.sin();
    pts.iter()
        .map(|p| {
            let dx = p.x - cx;
            let dy = p.y - cy;
            Point::new(cx + dx * cos_a - dy * sin_a, cy + dx * sin_a + dy * cos_a)
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rectangle_area() {
        let (pts, area) = calc_rectangle(0.0, 0.0, 1.0, 2.0, 0.1, 0.1);
        assert_eq!(pts.len(), 4);
        assert!((area - (0.8 * 1.8)).abs() < 1e-6);
    }

    #[test]
    fn test_array_slicing_45() {
        let slices = calc_array_slicing(0.0, 0.0, 4.0, 4.0, 2, 2, 0.3, true);
        assert!(!slices.is_empty());
        for (pts, area) in slices {
            assert!(pts.len() >= 3);
            assert!(area > 0.0);
        }
    }
}

