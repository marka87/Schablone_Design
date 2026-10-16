use crate::model::{Aperture, Point};
use rstar::{RTree, RTreeObject, AABB};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeViolation {
    pub ap1_id: usize,
    pub ap2_id: usize,
    pub pkg1: String,
    pub pkg2: String,
    pub distance: f64,
    pub p1: Point,
    pub p2: Point,
    pub severity: String, // "CRIT" | "WARN"
}

#[derive(Debug, Clone)]
pub struct ApertureRTreeItem {
    pub id: usize,
    pub index: usize,
    pub pkg: String,
    pub env: AABB<[f64; 2]>,
}

impl RTreeObject for ApertureRTreeItem {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

/// Ermittelt die Bounding Box einer Apertur.
fn aperture_bounds(ap: &Aperture) -> (f64, f64, f64, f64) {
    if !ap.points.is_empty() {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for p in &ap.points {
            if p.x < min_x { min_x = p.x; }
            if p.x > max_x { max_x = p.x; }
            if p.y < min_y { min_y = p.y; }
            if p.y > max_y { max_y = p.y; }
        }
        (min_x, min_y, max_x, max_y)
    } else {
        (
            ap.cx - ap.width / 2.0,
            ap.cy - ap.height / 2.0,
            ap.cx + ap.width / 2.0,
            ap.cy + ap.height / 2.0,
        )
    }
}

/// Berechnet den kürzesten euklidischen Abstand zwischen zwei Bounding Boxes und gibt Näherungspunkte zurück.
fn bbox_distance_with_points(
    b1: (f64, f64, f64, f64),
    b2: (f64, f64, f64, f64),
) -> (f64, Point, Point) {
    let (min_x1, min_y1, max_x1, max_y1) = b1;
    let (min_x2, min_y2, max_x2, max_y2) = b2;

    let (dx, p1_x, p2_x) = if max_x1 < min_x2 {
        (min_x2 - max_x1, max_x1, min_x2)
    } else if max_x2 < min_x1 {
        (min_x1 - max_x2, min_x1, max_x2)
    } else {
        let overlap_x = (min_x1.max(min_x2) + max_x1.min(max_x2)) / 2.0;
        (0.0, overlap_x, overlap_x)
    };

    let (dy, p1_y, p2_y) = if max_y1 < min_y2 {
        (min_y2 - max_y1, max_y1, min_y2)
    } else if max_y2 < min_y1 {
        (min_y1 - max_y2, min_y1, max_y2)
    } else {
        let overlap_y = (min_y1.max(min_y2) + max_y1.min(max_y2)) / 2.0;
        (0.0, overlap_y, overlap_y)
    };

    let dist = (dx * dx + dy * dy).sqrt();
    (dist, Point::new(p1_x, p1_y), Point::new(p2_x, p2_y))
}

/// Führt eine DFM-Stegbreitenprüfung (Web Check) auf benachbarten Schablonenöffnungen aus.
/// Standard-Grenzwert: 0.11 mm (Warnung vor Laserschneidfehlern und Stegbruch).
pub fn check_stencil_bridges(apertures: &[Aperture], min_bridge: f64) -> Vec<BridgeViolation> {
    let limit = if min_bridge > 0.0 { min_bridge } else { 0.11 };
    if apertures.len() < 2 {
        return Vec::new();
    }

    // 1. R-Tree aus den Apertur-Bounding-Boxes aufbauen
    let mut items = Vec::with_capacity(apertures.len());
    let mut bounds = Vec::with_capacity(apertures.len());

    for (idx, ap) in apertures.iter().enumerate() {
        let b = aperture_bounds(ap);
        bounds.push(b);
        items.push(ApertureRTreeItem {
            id: ap.id,
            index: idx,
            pkg: ap.package_name.clone(),
            env: AABB::from_corners([b.0, b.1], [b.2, b.3]),
        });
    }

    let tree = RTree::bulk_load(items);
    let mut violations = Vec::new();

    // 2. Schnelle Nachbarschaftssuche über R-Tree
    for (i, ap1) in apertures.iter().enumerate() {
        let b1 = bounds[i];
        let search_env = AABB::from_corners(
            [b1.0 - limit, b1.1 - limit],
            [b1.2 + limit, b1.3 + limit],
        );

        for neighbor in tree.locate_in_envelope(&search_env) {
            let j = neighbor.index;
            // Eindeutige Paare betrachten (j > i)
            if j <= i {
                continue;
            }

            let b2 = bounds[j];
            let (dist, p1, p2) = bbox_distance_with_points(b1, b2);

            if dist < limit {
                let ap2 = &apertures[j];
                let severity = if dist < 0.08 { "CRIT" } else { "WARN" };
                violations.push(BridgeViolation {
                    ap1_id: ap1.id,
                    ap2_id: ap2.id,
                    pkg1: ap1.package_name.clone(),
                    pkg2: ap2.package_name.clone(),
                    distance: dist,
                    p1,
                    p2,
                    severity: severity.to_string(),
                });
            }
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_check_bridge_violation() {
        let ap1 = Aperture {
            id: 1,
            pad_id: Some(1),
            package_name: "SOIC-8".into(),
            pin: 1,
            cx: 0.0,
            cy: 0.0,
            width: 0.4,
            height: 1.2,
            points: vec![],
            area: 0.48,
            shape_type: "rectangle".into(),
            is_modified: false,
            manual_override: false,
            ..Default::default()
        };

        // ap2 liegt nur 0.09 mm neben ap1 (Abstand < 0.11 mm)
        let ap2 = Aperture {
            id: 2,
            pad_id: Some(2),
            package_name: "SOIC-8".into(),
            pin: 2,
            cx: 0.49, // 0.2 + 0.09 + 0.2 = 0.49
            cy: 0.0,
            width: 0.4,
            height: 1.2,
            points: vec![],
            area: 0.48,
            shape_type: "rectangle".into(),
            is_modified: false,
            manual_override: false,
            ..Default::default()
        };

        let violations = check_stencil_bridges(&[ap1, ap2], 0.11);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].distance < 0.10);
    }
}

