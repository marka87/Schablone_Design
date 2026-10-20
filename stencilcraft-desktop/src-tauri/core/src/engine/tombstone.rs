//! Grabstein-Risiko (Tombstoning) bei 2-Pin-Chips.
//!
//! Hängt ein Pad an einer großen Kupferfläche (Masse/Versorgung) und das andere nur an einer
//! dünnen Leiterbahn, schmilzt das Lot ungleich auf und das Bauteil kann sich aufstellen.
//! Die Erkennung bewertet je Pad die thermische Anbindung auf der Außenlage und schlägt eine
//! Bauteilregel vor (Standard: Hausform, Spitze nach außen). Übernahme erfolgt als Empfehlung
//! oder – je Kunde einschaltbar – automatisch.

use super::geometry::{bounds, point_in_polygon, polygon_area};
use super::types::*;
use super::StencilEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Einstellungen (je Kunde in der Bibliothek gespeichert).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TombstoneConfig {
    /// Empfehlungen beim Laden automatisch als Bauteilregeln übernehmen
    pub auto_apply: bool,
    /// Kupferfläche ab dieser Größe (mm²) gilt als Wärmesenke
    pub min_pour_area: f64,
    /// Leiterbahnbreiten-Verhältnis stark/schwach, ab dem ohne Fläche gewarnt wird
    pub trace_ratio: f64,
    /// Nur Chips bis zu dieser Padlänge (mm) prüfen (0201 … 0805)
    pub max_pad_size: f64,
    /// Vorgeschlagene Regel
    pub rule: PackageRule,
}

impl Default for TombstoneConfig {
    fn default() -> Self {
        let mut rule = PackageRule::with("inverted_house_builder", "Grabstein-Schutz (Haus, Spitze nach außen)");
        rule.house_cut_pct = 30.0;
        rule.undersize_w = 0.025;
        rule.undersize_h = 0.025;
        rule.corner_radius = 0.05;
        TombstoneConfig { auto_apply: false, min_pour_area: 2.0, trace_ratio: 3.0, max_pad_size: 1.6, rule }
    }
}

impl TombstoneConfig {
    pub fn apply_map(&mut self, m: &Map<String, Value>) {
        let f = |k: &str| m.get(k).and_then(val_f64);
        if let Some(b) = m.get("auto_apply").and_then(val_bool) {
            self.auto_apply = b;
        }
        if let Some(v) = f("min_pour_area") { self.min_pour_area = v.clamp(0.1, 1000.0) }
        if let Some(v) = f("trace_ratio") { self.trace_ratio = v.clamp(1.2, 50.0) }
        if let Some(v) = f("max_pad_size") { self.max_pad_size = v.clamp(0.2, 5.0) }
        if let Some(r) = m.get("rule").and_then(|v| v.as_object()) {
            self.rule.apply_map(r);
        }
    }
}

/// Thermische Anbindung eines Pads auf der Außenlage.
#[derive(Debug, Clone, Copy, Default)]
pub struct PadLink {
    /// Fläche (mm²) der größten Kupferfläche, in der das Pad liegt (0 = keine)
    pub pour_area: f64,
    /// Summe der Breiten aller am Pad endenden Leiterbahnen (mm)
    pub trace_w: f64,
    pub traces: usize,
    /// Durchkontaktierungen im Pad (führen Wärme in Innenlagen ab)
    pub vias: usize,
}

impl PadLink {
    fn strong(&self, cfg: &TombstoneConfig) -> bool {
        self.pour_area >= cfg.min_pour_area
    }
}

impl StencilEngine {
    /// Merkt sich Kupferflächen und Leiterbahnen der Außenlage (beim Import je Seite).
    pub fn set_copper_geometry(&mut self, surfaces: Vec<Vec<Pt>>, traces: Vec<[f64; 5]>) {
        let min = self.tombstone.min_pour_area.min(1.0);
        self.copper_pours = surfaces.into_iter().filter(|s| s.len() >= 3 && polygon_area(s) >= min).collect();
        self.copper_traces = traces;
    }

    fn pad_link(&self, b: [f64; 4]) -> PadLink {
        let (cx, cy) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        let mut link = PadLink::default();
        for poly in &self.copper_pours {
            let (x0, y0, x1, y1) = bounds(poly);
            if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
                continue;
            }
            if point_in_polygon((cx, cy), poly) {
                link.pour_area = link.pour_area.max(polygon_area(poly));
            }
        }
        let tol = 0.03;
        let inside = |x: f64, y: f64| x >= b[0] - tol && x <= b[2] + tol && y >= b[1] - tol && y <= b[3] + tol;
        for t in &self.copper_traces {
            // Nur Bahnen, die am Pad enden und aus ihm herausführen
            let (a, e) = (inside(t[0], t[1]), inside(t[2], t[3]));
            if a != e {
                link.trace_w += t[4];
                link.traces += 1;
            }
        }
        link.vias = self.drills.iter().filter(|d| inside(d.0, d.1)).count();
        link
    }

    /// Bauteile mit Grabstein-Risiko auf der aktiven Seite.
    pub fn tombstone_candidates(&self) -> Vec<Value> {
        let cfg = &self.tombstone;
        let mut by_comp: std::collections::BTreeMap<&str, Vec<&ApertureItem>> = Default::default();
        for a in self.apertures.iter().filter(|a| !a.component_id.is_empty()) {
            by_comp.entry(a.component_id.as_str()).or_default().push(a);
        }
        let mut out = Vec::new();
        for (comp, aps) in by_comp {
            if aps.len() != 2 {
                continue;
            }
            let pkg = self.packages.get(&self.resolve_alias(&aps[0].package_name));
            if pkg.is_some_and(|p| p.pads_relative.len() > 2 || p.group == "THT") {
                continue;
            }
            let boxes: Vec<[f64; 4]> = aps
                .iter()
                .map(|a| a.copper_ref.as_ref().map_or(a.bbox, |c| [c.x0, c.y0, c.x1, c.y1]))
                .collect();
            if boxes.iter().any(|b| (b[2] - b[0]).max(b[3] - b[1]) > cfg.max_pad_size) {
                continue;
            }
            let l = [self.pad_link(boxes[0]), self.pad_link(boxes[1])];
            let (s0, s1) = (l[0].strong(cfg), l[1].strong(cfg));
            let reason = if s0 != s1 {
                Some("Ein Pad liegt in einer Kupferfläche, das andere nicht".to_string())
            } else if !s0 && !s1 {
                // Via im Pad zählt wie eine 0.3-mm-Bahn; Pads ohne erkennbare Anbindung nicht bewerten
                let eff = |k: &PadLink| k.trace_w + 0.3 * k.vias as f64;
                let known = l.iter().all(|k| k.traces + k.vias > 0);
                let (w0, w1) = (eff(&l[0]).max(0.05), eff(&l[1]).max(0.05));
                let ratio = w0.max(w1) / w0.min(w1);
                (known && ratio >= cfg.trace_ratio).then(|| format!("Leiterbahnbreiten {:.2} / {:.2} mm (×{ratio:.1})", l[0].trace_w, l[1].trace_w))
            } else {
                None
            };
            let Some(reason) = reason else { continue };
            let score = |k: &PadLink| k.pour_area * 10.0 + k.trace_w + 0.3 * k.vias as f64;
            let weak = if score(&l[0]) <= score(&l[1]) { 0 } else { 1 };
            let (cx, cy) = ((aps[0].cx + aps[1].cx) / 2.0, (aps[0].cy + aps[1].cy) / 2.0);
            out.push(json!({
                "component": comp,
                "package": aps[0].package_name,
                "x": r3(cx), "y": r3(cy),
                "reason": reason,
                "weak_pin": aps[weak].pin_number,
                "pins": aps.iter().zip(l.iter()).map(|(a, k)| json!({
                    "pin": a.pin_number, "id": a.id, "pour_area": r3(k.pour_area), "trace_w": r3(k.trace_w), "traces": k.traces, "vias": k.vias,
                })).collect::<Vec<_>>(),
                "has_rule": self.component_rules.contains_key(comp),
            }));
        }
        out
    }

    pub fn tombstone_json(&self) -> Value {
        let c = self.tombstone_candidates();
        let open = c.iter().filter(|v| !v["has_rule"].as_bool().unwrap_or(false)).count();
        json!({"candidates": c, "open": open, "config": self.tombstone, "copper_data": !self.copper_pours.is_empty() || !self.copper_traces.is_empty()})
    }

    /// Übernimmt die vorgeschlagene Regel als Bauteilregel (leer = alle offenen Kandidaten).
    pub fn apply_tombstone(&mut self, components: &[String]) -> usize {
        let targets: Vec<String> = if components.is_empty() {
            self.tombstone_candidates()
                .iter()
                .filter(|v| !v["has_rule"].as_bool().unwrap_or(false))
                .filter_map(|v| v["component"].as_str().map(String::from))
                .collect()
        } else {
            components.to_vec()
        };
        let rule = self.tombstone.rule.clone();
        let mut ids = Vec::new();
        for c in &targets {
            self.component_rules.insert(c.clone(), rule.clone());
            ids.extend(self.apertures.iter().filter(|a| &a.component_id == c).map(|a| a.id));
        }
        self.regenerate_ids(&ids);
        targets.len()
    }

    /// Beim Laden: bei eingeschalteter Automatik offene Kandidaten übernehmen.
    pub fn auto_apply_tombstone(&mut self) -> usize {
        if self.tombstone.auto_apply { self.apply_tombstone(&[]) } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(e: &mut StencilEngine, comp: &str, x: f64) {
        for (i, dx) in [(-0.5f64, -0.25), (0.5, 0.25)].iter().enumerate() {
            let mut a = ApertureItem::new(e.apertures.len(), "rectangle", x + dx.1, 0.0, 0.3, 0.3);
            a.component_id = comp.into();
            a.package_name = "R0402".into();
            a.pin_number = i as i64 + 1;
            e.apertures.push(a);
            let _ = dx.0;
        }
    }

    #[test]
    fn detects_pour_asymmetry() {
        let mut e = StencilEngine::new(0.12, std::env::temp_dir().join("sc_tomb_test"));
        e.apertures.clear();
        chip(&mut e, "R1", 0.0); // linkes Pad in großer Fläche
        chip(&mut e, "R2", 10.0); // beide Pads frei, gleiche Bahnen
        let pour = vec![(-5.0, -2.0), (-0.1, -2.0), (-0.1, 2.0), (-5.0, 2.0)];
        let traces = vec![[0.25, 0.0, 2.0, 0.0, 0.1], [9.75, 0.0, 8.0, 0.0, 0.1], [10.25, 0.0, 12.0, 0.0, 0.1]];
        e.set_copper_geometry(vec![pour], traces);
        let c = e.tombstone_candidates();
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!(c[0]["component"], "R1");
        assert_eq!(c[0]["weak_pin"], 2);
        // Übernehmen → Bauteilregel mit Hausform nach außen
        assert_eq!(e.apply_tombstone(&[]), 1);
        assert_eq!(e.component_rules["R1"].rule_type, "inverted_house_builder");
        assert!(e.tombstone_candidates()[0]["has_rule"].as_bool().unwrap());
    }

    #[test]
    fn detects_trace_ratio() {
        let mut e = StencilEngine::new(0.12, std::env::temp_dir().join("sc_tomb_test2"));
        e.apertures.clear();
        chip(&mut e, "C1", 0.0);
        let traces = vec![[-0.25, 0.0, -3.0, 0.0, 0.6], [0.25, 0.0, 3.0, 0.0, 0.1]];
        e.set_copper_geometry(vec![], traces);
        let c = e.tombstone_candidates();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["weak_pin"], 2);
    }
}
