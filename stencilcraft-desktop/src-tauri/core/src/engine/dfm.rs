//! DFM-Prüfungen: IPC-7525 (Area/Aspect Ratio, 5-Ball-Regel, Stufenkantenabstand),
//! Schablonensteg-Prüfung (R-Tree + exakte Polygonabstände, parallel), Pin-in-Paste, Auslösekalkulator.

use super::geometry::{circle_pts, polygon_area, polygon_distance, polygon_perimeter, rect_pts};
use super::types::*;
use super::StencilEngine;
use rayon::prelude::*;
use rstar::{RTree, RTreeObject, AABB};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::f64::consts::PI;

/// Fläche, Umfang, kleinste Kantenlänge einer Öffnung.
fn slice_metrics(s: &Slice) -> (f64, f64, f64) {
    let min_dim = s.w.min(s.h);
    if s.is_circle() {
        (PI * (s.w / 2.0).powi(2), PI * s.w, s.w)
    } else if let Some(pts) = s.polygon_pts() {
        (polygon_area(pts), polygon_perimeter(pts), min_dim)
    } else {
        (s.w * s.h, 2.0 * (s.w + s.h), min_dim)
    }
}

pub fn grain_diameter(paste_type: &str) -> f64 {
    match paste_type {
        "type3" => 0.045,
        "type5" => 0.025,
        _ => 0.038,
    }
}

/// Übliche Blechdicken (mm) für Empfehlungen und die Dicken-Übersicht.
pub const STD_THICKNESSES: [f64; 7] = [0.08, 0.10, 0.11, 0.12, 0.125, 0.13, 0.15];

/// Größte Standarddicke ≤ `t_max` (sonst die kleinste Standarddicke).
pub fn recommended_step(t_max: f64) -> f64 {
    STD_THICKNESSES.iter().copied().filter(|t| *t <= t_max + 1e-9).fold(STD_THICKNESSES[0], f64::max)
}

/// Area Ratio und Aspect Ratio bei 1 mm Dicke plus kleinste Öffnungsbreite.
/// Beide Kennzahlen sind umgekehrt proportional zur Dicke: Wert(t) = Wert(1 mm) / t.
fn ratios_at_unit_thickness(item: &ApertureItem) -> (f64, f64, f64) {
    match item.custom_slices.as_ref().filter(|s| !s.is_empty()) {
        Some(slices) => {
            let (mut ar, mut aspect, mut min_dim) = (f64::INFINITY, f64::INFINITY, f64::INFINITY);
            for s in slices {
                let (a, p, d) = slice_metrics(s);
                if p > 0.0 {
                    ar = ar.min(a / p);
                }
                aspect = aspect.min(s.w.min(s.h));
                min_dim = min_dim.min(d);
            }
            (if ar.is_finite() { ar } else { 0.0 }, aspect, min_dim)
        }
        None if item.shape_type == "circle" => (item.width / 4.0, item.width, item.width),
        None => (
            if item.perimeter > 0.0 { item.area / item.perimeter } else { 0.0 },
            item.width.min(item.height),
            item.width.min(item.height),
        ),
    }
}

impl StencilEngine {
    /// Größte Blechdicke (mm), bei der die Öffnung Area- und Aspect-Ratio-Grenze noch einhält.
    pub fn max_thickness_for(&self, item: &ApertureItem) -> f64 {
        let (ar1, asp1, _) = ratios_at_unit_thickness(item);
        let by_ar = if self.ipc_min_ar > 0.0 { ar1 / self.ipc_min_ar } else { f64::INFINITY };
        let by_asp = if self.ipc_min_aspect > 0.0 { asp1 / self.ipc_min_aspect } else { f64::INFINITY };
        by_ar.min(by_asp)
    }

    fn status_at(&self, item: &ApertureItem, t: f64) -> &'static str {
        let (ar1, asp1, _) = ratios_at_unit_thickness(item);
        let (ar, asp) = (ar1 / t, asp1 / t);
        let lo = 1.0 - self.ipc_warn_margin;
        if ar >= self.ipc_min_ar && asp >= self.ipc_min_aspect {
            "OK"
        } else if ar >= self.ipc_min_ar * lo && asp >= self.ipc_min_aspect * lo {
            "WARN"
        } else {
            "CRIT"
        }
    }

    /// Übersicht: Pads außerhalb von Stufenzonen je Standarddicke und die begrenzenden Packages.
    pub fn thickness_overview(&self) -> Value {
        let free: Vec<&ApertureItem> = self.apertures.iter().filter(|a| self.get_effective_thickness(a.cx, a.cy).1.is_none()).collect();
        let steps: Vec<Value> = STD_THICKNESSES
            .iter()
            .map(|&t| {
                let (mut ok, mut warn, mut crit) = (0, 0, 0);
                for a in &free {
                    match self.status_at(a, t) {
                        "OK" => ok += 1,
                        "WARN" => warn += 1,
                        _ => crit += 1,
                    }
                }
                json!({"t": t, "ok": ok, "warn": warn, "crit": crit})
            })
            .collect();
        // Packages, die die Grunddicke begrenzen (kleinste zulässige Dicke zuerst)
        let mut by_pkg: HashMap<String, (f64, usize, usize)> = HashMap::new();
        for a in &free {
            let tm = self.max_thickness_for(a);
            let e = by_pkg.entry(if a.package_name.is_empty() { a.package_group.clone() } else { a.package_name.clone() }).or_insert((f64::INFINITY, 0, 0));
            e.0 = e.0.min(tm);
            e.1 += 1;
            if tm < self.base_thickness - 1e-9 {
                e.2 += 1;
            }
        }
        let mut limiting: Vec<(String, (f64, usize, usize))> = by_pkg.into_iter().filter(|(_, v)| v.0 < self.base_thickness - 1e-9).collect();
        limiting.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0));
        let recommended = STD_THICKNESSES
            .iter()
            .rev()
            .copied()
            .find(|&t| free.iter().all(|a| self.status_at(a, t) != "CRIT"))
            .unwrap_or(STD_THICKNESSES[0]);
        // Ideal: alle Pads grün (kein WARN, kein CRIT)
        let recommended_ok = STD_THICKNESSES
            .iter()
            .rev()
            .copied()
            .find(|&t| free.iter().all(|a| self.status_at(a, t) == "OK"))
            .unwrap_or(STD_THICKNESSES[0]);
        json!({
            "current": self.base_thickness,
            "pads": free.len(),
            "steps": steps,
            "recommended": recommended,
            "recommended_ok": recommended_ok,
            "limiting": limiting.iter().take(10).map(|(n, (t, total, below))| json!({
                "package": n, "t_max": r3(*t), "step": recommended_step(*t), "pads": total, "pads_below": below,
            })).collect::<Vec<_>>(),
        })
    }

    /// Stufenzone um ein Bauteil (alle Öffnungen) mit Rakel- und Querabstand zur Zonenkante.
    pub fn add_zone_around_component(&mut self, refdes: &str, thickness: f64) -> Option<StencilZone> {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for a in self.apertures.iter().filter(|a| a.component_id == refdes) {
            b = [b[0].min(a.bbox[0]), b[1].min(a.bbox[1]), b[2].max(a.bbox[2]), b[3].max(a.bbox[3])];
        }
        if !b[0].is_finite() {
            return None;
        }
        // Etwas mehr als der geforderte Abstand, damit keine Kantenwarnung entsteht
        let (along, across) = (self.squeegee_clearance + 0.05, self.transverse_clearance + 0.05);
        let (mx, my) = if self.squeegee_dir == "Y" { (across, along) } else { (along, across) };
        Some(self.add_zone_ext(b[0] - mx, b[1] - my, b[2] + mx, b[3] + my, thickness, &format!("Stufe {refdes}"), true))
    }

    /// IPC-7525 Kennzahlen einer Apertur unter ihrer effektiven Schablonendicke.
    pub fn calculate_aperture_ipc(&self, item: &ApertureItem) -> IpcResult {
        let (t, zone) = self.get_effective_thickness(item.cx, item.cy);

        let (ar, aspect, min_ap_dim) = match item.custom_slices.as_ref().filter(|s| !s.is_empty()) {
            Some(slices) => {
                // Kritischste Teilöffnung ist maßgeblich
                let mut ar = f64::INFINITY;
                let mut aspect = f64::INFINITY;
                let mut min_dim = f64::INFINITY;
                for s in slices {
                    let (a, p, d) = slice_metrics(s);
                    if p > 0.0 {
                        ar = ar.min(a / (p * t));
                    }
                    aspect = aspect.min(s.w.min(s.h) / t);
                    min_dim = min_dim.min(d);
                }
                (if ar.is_finite() { ar } else { 0.0 }, aspect, min_dim)
            }
            None if item.shape_type == "circle" => (item.width / (4.0 * t), item.width / t, item.width),
            None => (
                if item.perimeter > 0.0 { item.area / (item.perimeter * t) } else { 0.0 },
                item.width.min(item.height) / t,
                item.width.min(item.height),
            ),
        };

        let lo = 1.0 - self.ipc_warn_margin;
        let mut status = if ar >= self.ipc_min_ar && aspect >= self.ipc_min_aspect {
            "OK"
        } else if ar >= self.ipc_min_ar * lo && aspect >= self.ipc_min_aspect * lo {
            "WARN"
        } else {
            "CRIT"
        };

        // Abstand zur Stufenkante (in Rakelrichtung strenger)
        let mut clearance_warn = false;
        if let Some(z) = zone {
            let dx = (item.cx - z.x0).abs().min((item.cx - z.x1).abs());
            let dy = (item.cy - z.y0).abs().min((item.cy - z.y1).abs());
            let (along, across) = if self.squeegee_dir == "Y" { (dy, dx) } else { (dx, dy) };
            clearance_warn = along < self.squeegee_clearance || across < self.transverse_clearance;
        }

        // 5-Ball-Regel nach IPC-7525 (Kornanzahl je IPC-Klasse, rund/rechteckig getrennt)
        let round = item.shape_type == "circle" || item.custom_slices.as_ref().is_some_and(|s| !s.is_empty() && s.iter().all(|x| x.is_circle()));
        let balls = if round { self.balls_round } else { self.balls_rect };
        let min_required_w = balls * grain_diameter(&self.paste_type);
        let grain_warn = min_ap_dim < min_required_w;
        let grain_msg = if grain_warn {
            if status == "OK" {
                status = "WARN";
            }
            format!("{balls:.0}-Ball-Regel: {:.3}mm < {:.3}mm ({})", min_ap_dim, min_required_w, self.paste_type.to_uppercase())
        } else {
            format!("{balls:.0}-Ball-Regel OK ({:.3}mm >= {:.3}mm)", min_ap_dim, min_required_w)
        };

        IpcResult {
            effective_thickness: t,
            zone_id: zone.map(|z| z.id.clone()),
            area_ratio: r3(ar),
            aspect_ratio: r2(aspect),
            status,
            clearance_warn,
            min_ap_dim: r3(min_ap_dim),
            paste_type: self.paste_type.clone(),
            grain_warn,
            grain_status: if grain_warn { "WARN_GRAIN" } else { "OK" },
            grain_msg,
            min_required_w: r3(min_required_w),
        }
    }

    pub fn all_ipc(&self) -> Vec<IpcResult> {
        self.apertures.par_iter().map(|a| self.calculate_aperture_ipc(a)).collect()
    }

    pub fn get_panel_statistics(&self) -> Value {
        let ipcs = self.all_ipc();
        let (mut ok, mut warn, mut crit, mut grain) = (0usize, 0usize, 0usize, 0usize);
        let mut min_ar = f64::INFINITY;
        let mut groups: HashMap<&str, usize> = HashMap::new();
        let mut crit_groups: HashMap<&str, usize> = HashMap::new();
        for (ap, ipc) in self.apertures.iter().zip(&ipcs) {
            *groups.entry(&ap.package_group).or_default() += 1;
            if ipc.grain_warn {
                grain += 1;
            }
            match ipc.status {
                "OK" => ok += 1,
                "WARN" => warn += 1,
                _ => {
                    crit += 1;
                    *crit_groups.entry(&ap.package_group).or_default() += 1;
                }
            }
            min_ar = min_ar.min(ipc.area_ratio);
        }
        let mut top: Vec<(&str, usize)> = crit_groups.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let top_crit: Vec<Value> = top
            .into_iter()
            .take(5)
            .map(|(g, c)| json!({"group": g, "crit_count": c, "total": groups.get(g).copied().unwrap_or(0)}))
            .collect();

        json!({
            "total_apertures": self.apertures.len(),
            "ok_count": ok,
            "warn_count": warn,
            "crit_count": crit,
            "grain_warn_count": grain,
            "min_ar": if min_ar.is_finite() { r3(min_ar) } else { 0.0 },
            "base_thickness": self.base_thickness,
            "zones_count": self.zones.len(),
            "zones": self.zones.iter().map(StencilZone::to_json).collect::<Vec<_>>(),
            "ipc_mode": self.ipc_mode,
            "ipc_class": self.ipc_class,
            "ipc_profile": self.ipc_profile_label(),
            "ipc_min_ar": self.ipc_min_ar,
            "ipc_min_aspect": self.ipc_min_aspect,
            "paste_type": self.paste_type,
            "min_bridge_width": self.min_bridge_width,
            "bridge_violations_count": self.bridge_violations.len(),
            "squeegee_dir": self.squeegee_dir,
            "bounds": self.bounds.iter().map(|b| r2(*b)).collect::<Vec<_>>(),
            "top_critical_packages": top_crit,
        })
    }

    // -------------------------------------------------------------
    // Schablonensteg-Prüfung (Web / Bridge Check)
    // -------------------------------------------------------------
    pub fn check_stencil_bridges(&mut self, min_bridge: f64) -> Value {
        self.min_bridge_width = min_bridge.max(0.04);
        let limit = self.min_bridge_width;

        struct GeomItem<'a> {
            ap_id: usize,
            slice_idx: i64,
            pts: Vec<Pt>,
            pkg: &'a str,
        }
        let mut items: Vec<GeomItem> = Vec::with_capacity(self.apertures.len());
        for ap in &self.apertures {
            match ap.custom_slices.as_ref().filter(|s| !s.is_empty()) {
                Some(slices) => {
                    for (i, s) in slices.iter().enumerate() {
                        let pts = if let Some(p) = s.polygon_pts() {
                            p.clone()
                        } else if s.is_circle() {
                            circle_pts(s.cx, s.cy, s.w / 2.0, 48)
                        } else {
                            rect_pts(s.cx, s.cy, s.w, s.h)
                        };
                        items.push(GeomItem { ap_id: ap.id, slice_idx: i as i64, pts, pkg: &ap.package_name });
                    }
                }
                None => {
                    let pts = if ap.shape_type == "polygon" && ap.points.len() >= 3 {
                        ap.points.clone()
                    } else if ap.shape_type == "circle" {
                        circle_pts(ap.cx, ap.cy, ap.width / 2.0, 48)
                    } else {
                        rect_pts(ap.cx, ap.cy, ap.width, ap.height)
                    };
                    items.push(GeomItem { ap_id: ap.id, slice_idx: -1, pts, pkg: &ap.package_name });
                }
            }
        }

        struct Env {
            idx: usize,
            env: AABB<[f64; 2]>,
        }
        impl RTreeObject for Env {
            type Envelope = AABB<[f64; 2]>;
            fn envelope(&self) -> Self::Envelope {
                self.env
            }
        }
        let envs: Vec<AABB<[f64; 2]>> = items
            .iter()
            .map(|it| {
                let (x0, y0, x1, y1) = super::geometry::bounds(&it.pts);
                AABB::from_corners([x0, y0], [x1, y1])
            })
            .collect();
        let tree = RTree::bulk_load(envs.iter().enumerate().map(|(idx, e)| Env { idx, env: *e }).collect());

        let mut violations: Vec<BridgeViolation> = (0..items.len())
            .into_par_iter()
            .flat_map_iter(|i| {
                let it = &items[i];
                let (lo, hi) = (envs[i].lower(), envs[i].upper());
                let query = AABB::from_corners([lo[0] - limit, lo[1] - limit], [hi[0] + limit, hi[1] + limit]);
                let mut found = Vec::new();
                for other in tree.locate_in_envelope_intersecting(&query) {
                    let j = other.idx;
                    if j <= i {
                        continue;
                    }
                    let ot = &items[j];
                    let same_pad = it.ap_id == ot.ap_id && it.slice_idx >= 0 && ot.slice_idx >= 0;
                    let threshold = if same_pad { 0.08 } else { limit };
                    let (dist, p1, p2) = polygon_distance(&it.pts, &ot.pts);
                    if dist > 0.0005 && dist < threshold {
                        let severity = if dist < threshold * 0.75 { "CRIT" } else { "WARN" };
                        found.push(BridgeViolation {
                            id: 0,
                            ap_id_1: it.ap_id,
                            ap_id_2: ot.ap_id,
                            pkg_1: it.pkg.to_string(),
                            pkg_2: ot.pkg.to_string(),
                            dist: r4(dist),
                            min_bridge: r3(threshold),
                            severity: severity.into(),
                            p1: (r3(p1.0), r3(p1.1)),
                            p2: (r3(p2.0), r3(p2.1)),
                            msg: format!("Stegbreite {:.3} mm < {:.3} mm ({})", dist, threshold, severity),
                        });
                    }
                }
                found
            })
            .collect();

        violations.sort_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap_or(std::cmp::Ordering::Equal));
        for (i, v) in violations.iter_mut().enumerate() {
            v.id = i + 1;
        }
        self.bridge_violations = violations;
        json!({
            "status": "ok",
            "count": self.bridge_violations.len(),
            "min_bridge": self.min_bridge_width,
            "violations": &self.bridge_violations[..self.bridge_violations.len().min(200)],
        })
    }
}

// -------------------------------------------------------------
// Pin-in-Paste (THT Reflow)
// -------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
pub fn calculate_pip(
    d_drill: f64,
    pin_shape: &str,
    pin_w: f64,
    pin_h: f64,
    h_pcb: f64,
    metal_ratio: f64,
    v_meniscus: f64,
    stencil_thick: f64,
    overprint_shape: &str,
) -> Value {
    let t = stencil_thick;
    let a_pin = if matches!(pin_shape.to_lowercase().as_str(), "round" | "circle" | "rund") {
        PI * pin_w.powi(2) / 4.0
    } else {
        pin_w * pin_h
    };
    let a_drill = PI * d_drill.powi(2) / 4.0;
    let v_hole_net = ((a_drill - a_pin) * h_pcb).max(0.0);
    let v_men = if v_meniscus <= 0.0 {
        if v_hole_net > 0.0 { 0.20 * v_hole_net } else { 0.05 }
    } else {
        v_meniscus
    };
    let metal = (if metal_ratio <= 1.0 { metal_ratio } else { metal_ratio / 100.0 }).clamp(0.10, 1.0);
    let v_paste = (v_hole_net + v_men) / metal;
    let a_req = if t > 0.0 { v_paste / t } else { 0.0 };

    let sq = a_req.sqrt();
    let rect = json!({
        "shape": "rectangle",
        "w": r3(sq),
        "h": r3(sq),
        "area": r4(sq * sq),
        "overprint_factor": if a_drill > 0.0 { r2(sq * sq / a_drill) } else { 1.0 },
    });
    let d_inner = (d_drill + 0.10).max(0.2);
    let d_outer = (4.0 * a_req / PI + d_inner.powi(2)).sqrt();
    let donut = json!({
        "shape": "donut",
        "d_inner": r3(d_inner),
        "d_outer": r3(d_outer),
        "area": r4(PI / 4.0 * (d_outer.powi(2) - d_inner.powi(2))),
        "overprint_factor": if a_drill > 0.0 { r2(a_req / a_drill) } else { 1.0 },
    });
    let core = (d_drill + 0.20).max(0.4);
    let arm_w = core * 0.70;
    let core_area = core * core;
    let rem = (a_req - core_area).max(0.0);
    let arm_len = if arm_w > 0.0 { rem / (4.0 * arm_w) } else { 0.5 };
    let cross = json!({
        "shape": "cross",
        "core_size": r3(core),
        "arm_width": r3(arm_w),
        "arm_length": r3(arm_len),
        "total_span": r3(core + 2.0 * arm_len),
        "area": r4(core_area + 4.0 * arm_w * arm_len),
    });
    let selected = match overprint_shape {
        "donut" => donut.clone(),
        "cross" => cross.clone(),
        _ => rect.clone(),
    };
    json!({
        "d_drill": d_drill,
        "pin_shape": pin_shape,
        "pin_w": pin_w,
        "pin_h": pin_h,
        "h_pcb": h_pcb,
        "metal_ratio_pct": (metal * 1000.0).round() / 10.0,
        "stencil_thickness": t,
        "v_hole_net_mm3": r4(v_hole_net),
        "v_meniscus_mm3": r4(v_men),
        "v_paste_soll_mm3": r4(v_paste),
        "a_aperture_req_mm2": r4(a_req),
        "geometries": {"rectangle": rect, "donut": donut, "cross": cross},
        "selected_geometry": selected,
    })
}

// -------------------------------------------------------------
// Auslösekalkulator (Transfer Efficiency, analog Inhouse-Excel)
// -------------------------------------------------------------
pub fn calculate_transfer_calculator(width: f64, height: f64, unit: &str, thicknesses: &[f64]) -> Value {
    let is_mil = unit.eq_ignore_ascii_case("mil");
    let (w_mil, h_mil, w_mm, h_mm) = if is_mil {
        (width, height, width * 0.0254, height * 0.0254)
    } else {
        (width / 0.0254, height / 0.0254, width, height)
    };
    let area_mil = w_mil * h_mil;
    let perim_mil = 2.0 * (w_mil + h_mil);
    let area_mm = w_mm * h_mm;
    let perim_mm = 2.0 * (w_mm + h_mm);

    let mut baseline: Option<f64> = None;
    let mut results = Vec::new();
    for &t_um in thicknesses {
        let t_mil = t_um / 25.0;
        let t_mm = t_um / 1000.0;
        let wall_mil = perim_mil * t_mil;
        let wall_mm = perim_mm * t_mm;
        let ar = if wall_mil > 0.0 { area_mil / wall_mil } else { 0.0 };
        let eff_w = (w_mil - 0.1 * t_mil).max(0.0);
        let eff_l = (h_mil - 0.1 * t_mil).max(0.0);
        let theor = eff_w * eff_l * t_mil;
        let eta = if ar <= 0.7 {
            2.535 * ar.powf(3.426)
        } else if ar >= 1.0 {
            1.0
        } else {
            ar
        };
        let vol = theor * eta;
        let base = *baseline.get_or_insert(vol);
        let rel = if base > 0.0 { (vol / base * 100.0).round() } else { 100.0 };
        let status = if ar >= 0.70 { "OK" } else if ar >= 0.66 { "WARN" } else { "CRIT" };
        results.push(json!({
            "thickness_um": t_um.round() as i64,
            "thickness_mil": r2(t_mil),
            "area_mil2": r2(area_mil),
            "wall_mil2": r2(wall_mil),
            "area_mm2": r4(area_mm),
            "wall_mm2": r4(wall_mm),
            "area_ratio": r2(ar),
            "volume_mil3": r2(vol),
            "volume_nl": r3(vol * 0.000016387064),
            "transfer_pct": rel as i64,
            "status": status,
        }));
    }
    json!({
        "input_unit": unit,
        "width_mil": r2(w_mil),
        "height_mil": r2(h_mil),
        "width_mm": r3(w_mm),
        "height_mm": r3(h_mm),
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_volume_formula() {
        let r = calculate_pip(1.0, "round", 0.6, 0.6, 1.6, 0.5, 0.0, 0.12, "rectangle");
        // V_netto = (π/4·1² − π/4·0.6²)·1.6 = 0.8042 mm³
        assert!((r["v_hole_net_mm3"].as_f64().unwrap() - 0.8042).abs() < 1e-3);
        let a = r["a_aperture_req_mm2"].as_f64().unwrap();
        assert!((a - (0.8042 * 1.2 / 0.5 / 0.12)).abs() < 0.01);
    }

    #[test]
    fn transfer_calculator_rows() {
        let r = calculate_transfer_calculator(11.8, 11.8, "mil", &[80.0, 100.0, 120.0]);
        let rows = r["results"].as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["transfer_pct"], 100);
    }
}

#[cfg(test)]
mod thickness_tests {
    use super::*;

    #[test]
    fn max_thickness_bga_pad() {
        // BGA-Pad 0.23 × 0.23 mm, AR ≥ 0.66: t ≤ 0.23² / (4·0.23 · 0.66) = 0.0871 mm → 80 µm
        let e = StencilEngine::new(0.12, std::env::temp_dir().join("sc_thick_test"));
        let ap = ApertureItem::new(0, "rectangle", 0.0, 0.0, 0.23, 0.23);
        let tm = e.max_thickness_for(&ap);
        assert!((tm - 0.0871).abs() < 1e-3, "{tm}");
        assert!((recommended_step(tm) - 0.08).abs() < 1e-9);
        assert!((recommended_step(0.2) - 0.15).abs() < 1e-9);
        assert!((recommended_step(0.05) - 0.08).abs() < 1e-9);
    }
}
