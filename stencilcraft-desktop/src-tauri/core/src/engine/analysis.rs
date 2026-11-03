//! Stencil-Analyse als Tabelle: je Öffnung Fläche, Volumen, Pad-Abdeckung, Area/Aspect Ratio und Befunde.
//! Grundlage für Filter, Sprung zur Stelle und CSV-Export.

use super::dfm::slice_metrics;
use super::types::{r3, ApertureItem};
use super::StencilEngine;
use serde_json::{json, Value};
use std::f64::consts::PI;

/// Grenzen der Paste-zu-Pad-Abdeckung (%), außerhalb gilt die Öffnung als auffällig.
#[derive(Clone, Copy)]
pub struct CoverageLimits {
    pub min_pct: f64,
    pub max_pct: f64,
}

impl Default for CoverageLimits {
    fn default() -> Self {
        CoverageLimits { min_pct: 30.0, max_pct: 150.0 }
    }
}

/// Fläche der Öffnung (mm²); 0 bei „keine Öffnung“.
pub fn opening_area(ap: &ApertureItem) -> f64 {
    match &ap.custom_slices {
        Some(s) => s.iter().map(|x| slice_metrics(x).0).sum(),
        None => ap.area,
    }
}

/// Fläche des Pads, auf das gedruckt wird: Kupfer der Außenlage, sonst Maskenfreistellung.
pub fn pad_area(ap: &ApertureItem) -> Option<f64> {
    let p = ap.copper_ref.as_ref().or(ap.pad_ref.as_ref())?;
    let a = if p.pts.len() >= 3 {
        super::geometry::polygon_area(&p.pts)
    } else if p.round {
        PI * (p.w / 2.0).powi(2)
    } else {
        p.w * p.h
    };
    (a > 0.0).then_some(a)
}

impl StencilEngine {
    pub fn analysis_rows(&self, lim: CoverageLimits) -> Vec<Value> {
        let ipcs = self.all_ipc();
        self.apertures
            .iter()
            .zip(ipcs)
            .filter(|(ap, _)| ap.custom_slices.as_ref().map_or(true, |s| !s.is_empty()))
            .map(|(ap, ipc)| {
                let area = opening_area(ap);
                let pad = pad_area(ap);
                let cov = pad.map(|p| area / p * 100.0);
                let mut issues: Vec<&str> = Vec::new();
                if ipc.status != "OK" {
                    issues.push(if ipc.area_ratio < self.ipc_min_ar { "Area Ratio" } else if ipc.aspect_ratio < self.ipc_min_aspect { "Aspect Ratio" } else { "Grenzwertig" });
                }
                if ipc.grain_warn {
                    issues.push("Ball-Regel");
                }
                if ipc.via_warn {
                    issues.push("Bohrung in Paste");
                }
                if ipc.squeegee_warn {
                    issues.push("Quer zur Rakel");
                }
                if ipc.clearance_warn {
                    issues.push("Stufenkante");
                }
                if cov.is_some_and(|c| c < lim.min_pct) {
                    issues.push("Abdeckung zu klein");
                }
                if cov.is_some_and(|c| c > lim.max_pct) {
                    issues.push("Abdeckung zu groß");
                }
                let level = if ipc.status == "CRIT" { "CRIT" } else if ipc.status == "WARN" || !issues.is_empty() { "WARN" } else { "OK" };
                json!({
                    "id": ap.id,
                    "comp": ap.component_id,
                    "pin": ap.pin_number,
                    "pkg": ap.package_name,
                    "x": r3(ap.cx),
                    "y": r3(ap.cy),
                    "area": r3(area * 1000.0) / 1000.0,
                    "pad_area": pad.map(|p| (p * 1e4).round() / 1e4),
                    "coverage": cov.map(|c| (c * 10.0).round() / 10.0),
                    "thickness": ipc.effective_thickness,
                    // Volumen in mm³ (Öffnungsfläche × Blechdicke)
                    "volume": (area * ipc.effective_thickness * 1e5).round() / 1e5,
                    "ar": ipc.area_ratio,
                    "aspect": ipc.aspect_ratio,
                    "min_w": ipc.min_ap_dim,
                    "status": level,
                    "issues": issues,
                    "manual": ap.is_manually_overridden,
                })
            })
            .collect()
    }
}

/// CSV mit Semikolon und Dezimalkomma (öffnet direkt in deutschem Excel).
pub fn analysis_csv(rows: &[Value]) -> String {
    let num = |v: &Value| v.as_f64().map(|f| format!("{f}").replace('.', ",")).unwrap_or_default();
    let txt = |v: &Value| v.as_str().unwrap_or("").replace(';', ",");
    let mut out = String::from("Id;Bauteil;Pin;Package;X (mm);Y (mm);Fläche (mm²);Padfläche (mm²);Abdeckung (%);Dicke (mm);Volumen (mm³);Area Ratio;Aspect Ratio;Min. Breite (mm);Status;Befunde\n");
    for r in rows {
        let issues: Vec<&str> = r["issues"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        out.push_str(&format!(
            "{};{};{};{};{};{};{};{};{};{};{};{};{};{};{};{}\n",
            r["id"], txt(&r["comp"]), r["pin"], txt(&r["pkg"]), num(&r["x"]), num(&r["y"]), num(&r["area"]), num(&r["pad_area"]),
            num(&r["coverage"]), num(&r["thickness"]), num(&r["volume"]), num(&r["ar"]), num(&r["aspect"]), num(&r["min_w"]),
            txt(&r["status"]), issues.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::{PadBox, Slice};
    use std::path::PathBuf;

    #[test]
    fn rows_report_coverage_volume_and_issues() {
        let mut e = StencilEngine::new(0.1, PathBuf::from("."));
        let mut a = ApertureItem::new(0, "rectangle", 0.0, 0.0, 1.0, 1.0);
        a.copper_ref = Some(PadBox::new(-0.5, -0.5, 0.5, 0.5));
        let mut b = a.clone();
        b.id = 1;
        b.custom_slices = Some(vec![Slice::rect(0.0, 0.0, 0.5, 0.5)]); // 25 % Abdeckung
        let mut gone = a.clone();
        gone.id = 2;
        gone.custom_slices = Some(Vec::new()); // keine Öffnung: nicht in der Tabelle
        e.apertures = vec![a, b, gone];
        let rows = e.analysis_rows(CoverageLimits::default());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["coverage"], 100.0);
        assert!((rows[0]["volume"].as_f64().unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(rows[0]["status"], "OK");
        assert_eq!(rows[1]["coverage"], 25.0);
        assert_eq!(rows[1]["status"], "WARN");
        assert!(rows[1]["issues"].as_array().unwrap().iter().any(|i| i == "Abdeckung zu klein"));
        let csv = analysis_csv(&rows);
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.lines().nth(2).unwrap().contains(";25;"), "{csv}");
    }
}
