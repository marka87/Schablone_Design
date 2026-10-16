//! Geometrie-Grundfunktionen: Flächen, Umfang, konvexes Clipping, Abstände, Kreis-/Bogenapproximation.

use super::types::Pt;
use std::f64::consts::PI;

pub fn bounds(pts: &[Pt]) -> (f64, f64, f64, f64) {
    let mut b = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for &(x, y) in pts {
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x);
        b.3 = b.3.max(y);
    }
    b
}

pub fn signed_area(pts: &[Pt]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut s = 0.0;
    for i in 0..n {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % n];
        s += x0 * y1 - x1 * y0;
    }
    s / 2.0
}

pub fn polygon_area(pts: &[Pt]) -> f64 {
    signed_area(pts).abs()
}

pub fn polygon_perimeter(pts: &[Pt]) -> f64 {
    let n = pts.len();
    if n < 2 {
        return 0.0;
    }
    (0..n)
        .map(|i| {
            let (x0, y0) = pts[i];
            let (x1, y1) = pts[(i + 1) % n];
            (x1 - x0).hypot(y1 - y0)
        })
        .sum()
}

pub fn centroid(pts: &[Pt]) -> Pt {
    let a = signed_area(pts);
    if a.abs() < 1e-12 {
        let n = pts.len().max(1) as f64;
        return (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    }
    let n = pts.len();
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..n {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[(i + 1) % n];
        let f = x0 * y1 - x1 * y0;
        cx += (x0 + x1) * f;
        cy += (y0 + y1) * f;
    }
    (cx / (6.0 * a), cy / (6.0 * a))
}

pub fn rect_pts(cx: f64, cy: f64, w: f64, h: f64) -> Vec<Pt> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    vec![(cx - hw, cy - hh), (cx + hw, cy - hh), (cx + hw, cy + hh), (cx - hw, cy + hh)]
}

/// Achteck mit 45°-gebrochenen Ecken (Approximation der Eckverrundung wie im Web-Original).
pub fn rounded_rect_pts(cx: f64, cy: f64, w: f64, h: f64, r: f64) -> Vec<Pt> {
    vec![
        (cx - w / 2.0 + r, cy + h / 2.0),
        (cx + w / 2.0 - r, cy + h / 2.0),
        (cx + w / 2.0, cy + h / 2.0 - r),
        (cx + w / 2.0, cy - h / 2.0 + r),
        (cx + w / 2.0 - r, cy - h / 2.0),
        (cx - w / 2.0 + r, cy - h / 2.0),
        (cx - w / 2.0, cy - h / 2.0 + r),
        (cx - w / 2.0, cy + h / 2.0 - r),
    ]
}

pub fn circle_pts(cx: f64, cy: f64, r: f64, n: usize) -> Vec<Pt> {
    (0..n)
        .map(|i| {
            let a = 2.0 * PI * i as f64 / n as f64;
            (cx + r * a.cos(), cy + r * a.sin())
        })
        .collect()
}

/// Langloch (Obround / Oval) als Polygon.
pub fn stadium_pts(cx: f64, cy: f64, w: f64, h: f64, seg: usize) -> Vec<Pt> {
    let r = w.min(h) / 2.0;
    let mut pts = Vec::with_capacity(seg * 2 + 2);
    if w >= h {
        let off = w / 2.0 - r;
        for i in 0..=seg {
            let a = -PI / 2.0 + PI * i as f64 / seg as f64;
            pts.push((cx + off + r * a.cos(), cy + r * a.sin()));
        }
        for i in 0..=seg {
            let a = PI / 2.0 + PI * i as f64 / seg as f64;
            pts.push((cx - off + r * a.cos(), cy + r * a.sin()));
        }
    } else {
        let off = h / 2.0 - r;
        for i in 0..=seg {
            let a = PI * i as f64 / seg as f64;
            pts.push((cx + r * a.cos(), cy + off + r * a.sin()));
        }
        for i in 0..=seg {
            let a = PI + PI * i as f64 / seg as f64;
            pts.push((cx + r * a.cos(), cy - off + r * a.sin()));
        }
    }
    pts
}

/// Rotiert Punkte um (ox, oy) um `deg` Grad (mathematisch positiv = gegen den Uhrzeigersinn).
pub fn rotate_pts(pts: &[Pt], ox: f64, oy: f64, deg: f64) -> Vec<Pt> {
    if deg.abs() < 1e-9 {
        return pts.to_vec();
    }
    let (s, c) = deg.to_radians().sin_cos();
    pts.iter()
        .map(|&(x, y)| {
            let (dx, dy) = (x - ox, y - oy);
            (ox + dx * c - dy * s, oy + dx * s + dy * c)
        })
        .collect()
}

/// Bogen von Start nach Ende um Zentrum als Polylinie (inklusive Endpunkt, exklusive Startpunkt).
pub fn arc_points(start: Pt, end: Pt, center: Pt, clockwise: bool) -> Vec<Pt> {
    let r = (start.0 - center.0).hypot(start.1 - center.1);
    let a0 = (start.1 - center.1).atan2(start.0 - center.0);
    let mut a1 = (end.1 - center.1).atan2(end.0 - center.0);
    if clockwise {
        while a1 >= a0 - 1e-9 {
            a1 -= 2.0 * PI;
        }
    } else {
        while a1 <= a0 + 1e-9 {
            a1 += 2.0 * PI;
        }
    }
    let sweep = a1 - a0;
    let n = ((sweep.abs() / (PI / 18.0)).ceil() as usize).clamp(2, 72);
    let mut pts = Vec::with_capacity(n);
    for i in 1..n {
        let a = a0 + sweep * i as f64 / n as f64;
        pts.push((center.0 + r * a.cos(), center.1 + r * a.sin()));
    }
    pts.push(end);
    pts
}

/// Sutherland-Hodgman Clipping an einer Halbebene: behält Punkte mit `nx*x + ny*y >= c`.
pub fn clip_halfplane(poly: &[Pt], nx: f64, ny: f64, c: f64) -> Vec<Pt> {
    let n = poly.len();
    if n == 0 {
        return Vec::new();
    }
    let side = |p: Pt| nx * p.0 + ny * p.1 - c;
    let mut out = Vec::with_capacity(n + 2);
    for i in 0..n {
        let cur = poly[i];
        let prev = poly[(i + n - 1) % n];
        let (sc, sp) = (side(cur), side(prev));
        if sc >= 0.0 {
            if sp < 0.0 {
                let t = sp / (sp - sc);
                out.push((prev.0 + t * (cur.0 - prev.0), prev.1 + t * (cur.1 - prev.1)));
            }
            out.push(cur);
        } else if sp >= 0.0 {
            let t = sp / (sp - sc);
            out.push((prev.0 + t * (cur.0 - prev.0), prev.1 + t * (cur.1 - prev.1)));
        }
    }
    out
}

/// Konvexe Hülle (Andrew Monotone Chain), gegen den Uhrzeigersinn.
pub fn convex_hull(points: &[Pt]) -> Vec<Pt> {
    let mut pts: Vec<Pt> = points.to_vec();
    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
    pts.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9);
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Pt, a: Pt, b: Pt| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<Pt> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

pub fn point_in_polygon(p: Pt, poly: &[Pt]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > p.1) != (yj > p.1) && p.0 < (xj - xi) * (p.1 - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Nächster Punkt auf Segment ab zu Punkt p.
fn closest_on_segment(p: Pt, a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-18 {
        return a;
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (a.0 + t * dx, a.1 + t * dy)
}

fn segments_intersect(a: Pt, b: Pt, c: Pt, d: Pt) -> bool {
    let orient = |p: Pt, q: Pt, r: Pt| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
    let (o1, o2, o3, o4) = (orient(a, b, c), orient(a, b, d), orient(c, d, a), orient(c, d, b));
    (o1 > 0.0) != (o2 > 0.0) && (o3 > 0.0) != (o4 > 0.0) && o1 != 0.0 && o2 != 0.0 && o3 != 0.0 && o4 != 0.0
}

/// Minimaler Abstand zwischen zwei Polygonen inkl. nächster Punkte.
/// Überlappende oder sich schneidende Polygone liefern Abstand 0.
pub fn polygon_distance(a: &[Pt], b: &[Pt]) -> (f64, Pt, Pt) {
    if a.is_empty() || b.is_empty() {
        return (f64::INFINITY, (0.0, 0.0), (0.0, 0.0));
    }
    if point_in_polygon(a[0], b) {
        return (0.0, a[0], a[0]);
    }
    if point_in_polygon(b[0], a) {
        return (0.0, b[0], b[0]);
    }
    let (na, nb) = (a.len(), b.len());
    let mut best = (f64::INFINITY, a[0], b[0]);
    for i in 0..na {
        let (a0, a1) = (a[i], a[(i + 1) % na]);
        for j in 0..nb {
            let (b0, b1) = (b[j], b[(j + 1) % nb]);
            if segments_intersect(a0, a1, b0, b1) {
                return (0.0, a0, a0);
            }
            for (p, s0, s1, p_on_a) in [(a0, b0, b1, true), (a1, b0, b1, true), (b0, a0, a1, false), (b1, a0, a1, false)] {
                let q = closest_on_segment(p, s0, s1);
                let d = (p.0 - q.0).hypot(p.1 - q.1);
                if d < best.0 {
                    best = if p_on_a { (d, p, q) } else { (d, q, p) };
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_and_perimeter_of_rectangle() {
        let r = rect_pts(0.0, 0.0, 2.0, 1.0);
        assert!((polygon_area(&r) - 2.0).abs() < 1e-9);
        assert!((polygon_perimeter(&r) - 6.0).abs() < 1e-9);
    }

    #[test]
    fn halfplane_clip_cuts_square_in_half() {
        let r = rect_pts(0.0, 0.0, 2.0, 2.0);
        let c = clip_halfplane(&r, 1.0, 0.0, 0.0);
        assert!((polygon_area(&c) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn distance_between_separate_squares() {
        let a = rect_pts(0.0, 0.0, 1.0, 1.0);
        let b = rect_pts(1.6, 0.0, 1.0, 1.0);
        let (d, p1, p2) = polygon_distance(&a, &b);
        assert!((d - 0.6).abs() < 1e-9);
        assert!((p1.0 - 0.5).abs() < 1e-9 && (p2.0 - 1.1).abs() < 1e-9);
        let c = rect_pts(0.5, 0.5, 1.0, 1.0);
        assert_eq!(polygon_distance(&a, &c).0, 0.0);
    }

    #[test]
    fn hull_of_rounded_square_is_convex() {
        let mut pts = circle_pts(0.0, 0.0, 1.0, 16);
        pts.push((0.0, 0.0));
        assert_eq!(convex_hull(&pts).len(), 16);
    }
}
