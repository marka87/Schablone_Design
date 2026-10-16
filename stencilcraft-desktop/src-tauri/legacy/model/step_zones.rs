use crate::model::{Aperture, Point};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    StepUp,
    StepDown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepZone {
    pub id: String,
    pub zone_type: StepType,
    pub thickness: f64, // z. B. 0.10 mm oder 0.15 mm
    pub points: Vec<Point>,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl StepZone {
    pub fn new_rect(id: &str, zone_type: StepType, thickness: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        let min_x = x0.min(x1);
        let max_x = x0.max(x1);
        let min_y = y0.min(y1);
        let max_y = y0.max(y1);

        let points = vec![
            Point::new(min_x, min_y),
            Point::new(max_x, min_y),
            Point::new(max_x, max_y),
            Point::new(min_x, max_y),
        ];

        Self {
            id: id.to_string(),
            zone_type,
            thickness,
            points,
            x0: min_x,
            y0: min_y,
            x1: max_x,
            y1: max_y,
        }
    }

    /// Berechnet den kürzesten Abstand eines Punktes zu den Begrenzungskanten der Stufenzone.
    pub fn distance_to_edge(&self, px: f64, py: f64) -> f64 {
        // Punkt-zu-Segment Distanz für alle 4 Kanten
        let mut min_d = f64::INFINITY;
        let n = self.points.len();
        for i in 0..n {
            let p1 = &self.points[i];
            let p2 = &self.points[(i + 1) % n];
            let d = point_to_segment_dist(px, py, p1.x, p1.y, p2.x, p2.y);
            if d < min_d {
                min_d = d;
            }
        }
        min_d
    }

    /// Prüft, ob ein Punkt innerhalb der Stufenzone liegt.
    pub fn contains_point(&self, px: f64, py: f64) -> bool {
        px >= self.x0 && px <= self.x1 && py >= self.y0 && py <= self.y1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneClearanceViolation {
    pub zone_id: String,
    pub aperture_id: usize,
    pub package_name: String,
    pub distance_to_step: f64,
    pub min_required_clearance: f64,
}

/// Prüft, ob Öffnungen näher als der geforderte Mindestabstand (Standard: 1.50 mm) an der Stufenkante liegen.
pub fn validate_zone_clearances(
    zones: &[StepZone],
    apertures: &[Aperture],
    min_clearance: f64,
) -> Vec<ZoneClearanceViolation> {
    let limit = if min_clearance > 0.0 { min_clearance } else { 1.50 };
    let mut violations = Vec::new();

    for zone in zones {
        for ap in apertures {
            let dist = zone.distance_to_edge(ap.cx, ap.cy);
            if dist < limit {
                violations.push(ZoneClearanceViolation {
                    zone_id: zone.id.clone(),
                    aperture_id: ap.id,
                    package_name: ap.package_name.clone(),
                    distance_to_step: dist,
                    min_required_clearance: limit,
                });
            }
        }
    }

    violations
}

fn point_to_segment_dist(px: f64, py: f64, x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-9 {
        return ((px - x1).powi(2) + (py - y1).powi(2)).sqrt();
    }
    let t = (((px - x1) * dx + (py - y1) * dy) / len_sq).clamp(0.0, 1.0);
    let proj_x = x1 + t * dx;
    let proj_y = y1 + t * dy;
    ((px - proj_x).powi(2) + (py - proj_y).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_zone_clearance() {
        let zone = StepZone::new_rect("ZONE_1", StepType::StepDown, 0.10, 0.0, 0.0, 20.0, 20.0);
        let ap = Aperture {
            id: 1,
            pad_id: Some(1),
            package_name: "0805".into(),
            component_refdes: "R1".into(),
            pin: 1,
            cx: 20.8, // 0.8 mm von der Kante entfernt (< 1.50 mm)
            cy: 10.0,
            width: 1.2,
            height: 1.3,
            facing_dir: "down".into(),
            rotation: 0.0,
            points: vec![],
            area: 1.56,
            shape_type: "rectangle".into(),
            is_modified: false,
            is_custom: false,
            manual_override: false,
        };

        let violations = validate_zone_clearances(&[zone], &[ap], 1.50);
        assert_eq!(violations.len(), 1);
        assert!(violations[0].distance_to_step < 1.0);
    }
}

