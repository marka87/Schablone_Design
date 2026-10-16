use crate::model::{Aperture, Point};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IpcStatus {
    Ok,
    Warn,
    Crit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResult {
    pub aperture_id: usize,
    pub area_ratio: f64,
    pub aspect_ratio: f64,
    pub status: IpcStatus,
    pub grain_warn: bool,
    pub grain_msg: Option<String>,
}

/// Berechnet den Umfang (Perimeter) eines Polygons.
pub fn polygon_perimeter(pts: &[Point]) -> f64 {
    let n = pts.len();
    if n < 2 {
        return 0.0;
    }
    let mut perim = 0.0;
    for i in 0..n {
        let j = (i + 1) % n;
        perim += pts[i].dist(&pts[j]);
    }
    perim
}

/// Berechnet Area Ratio & Aspect Ratio nach IPC-7525 für eine einzelne Apertur.
pub fn calculate_aperture_ipc(ap: &Aperture, stencil_thickness: f64, paste_type: &str) -> IpcResult {
    let t = if stencil_thickness > 0.0 { stencil_thickness } else { 0.120 };

    let perim = if ap.points.len() >= 3 {
        polygon_perimeter(&ap.points)
    } else {
        2.0 * (ap.width + ap.height)
    };

    let area_ratio = if perim > 0.0 && t > 0.0 {
        ap.area / (perim * t)
    } else {
        0.0
    };

    let min_dim = ap.width.min(ap.height);
    let aspect_ratio = if t > 0.0 { min_dim / t } else { 0.0 };

    let status = if area_ratio >= 0.66 && aspect_ratio >= 1.50 {
        IpcStatus::Ok
    } else if area_ratio >= 0.55 && aspect_ratio >= 1.30 {
        IpcStatus::Warn
    } else {
        IpcStatus::Crit
    };

    // IPC-7525 5-Ball Regel für Pastenkorn-Validierung
    let grain_size_mm = match paste_type.to_lowercase().as_str() {
        "type3" | "typ3" => 0.045,
        "type5" | "typ5" => 0.025,
        _ => 0.038, // Default Type 4
    };
    let min_width_req = 5.0 * grain_size_mm;
    let grain_warn = min_dim < min_width_req;
    let grain_msg = if grain_warn {
        Some(format!(
            "5-Ball-Regel verletzt: Öffnungsbreite ({:.3} mm) < Mindestbreite ({:.3} mm für {})",
            min_dim, min_width_req, paste_type
        ))
    } else {
        None
    };

    IpcResult {
        aperture_id: ap.id,
        area_ratio,
        aspect_ratio,
        status,
        grain_warn,
        grain_msg,
    }
}

/// Parallele Auswertung aller Aperturen über Rayon zur maximalen Auslastung moderner Multi-Core CPUs.
pub fn analyze_ipc_parallel(
    apertures: &[Aperture],
    stencil_thickness: f64,
    paste_type: &str,
) -> Vec<IpcResult> {
    apertures
        .par_iter()
        .map(|ap| calculate_aperture_ipc(ap, stencil_thickness, paste_type))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_calc() {
        let ap = Aperture {
            id: 1,
            pad_id: Some(1),
            package_name: "0805".into(),
            pin: 1,
            cx: 0.0,
            cy: 0.0,
            width: 1.2,
            height: 1.3,
            points: vec![
                Point::new(-0.6, -0.65),
                Point::new(0.6, -0.65),
                Point::new(0.6, 0.65),
                Point::new(-0.6, 0.65),
            ],
            area: 1.56,
            shape_type: "rectangle".into(),
            is_modified: false,
            manual_override: false,
            ..Default::default()
        };

        let res = calculate_aperture_ipc(&ap, 0.12, "type4");
        assert_eq!(res.status, IpcStatus::Ok);
        assert!(!res.grain_warn);
    }
}

