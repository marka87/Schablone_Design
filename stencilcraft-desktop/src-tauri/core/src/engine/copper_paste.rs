//! Bauteilpins ohne Pastenöffnung in der Kundenlage.
//!
//! PCB Investigator erzeugt bei der Regel "Package Group" Öffnungen für alle Bauteilpins aus dem
//! Kupfer, auch wenn die Pastenlage des Kunden dort nichts enthält (Testpads, Spark-Gaps, …).
//! Hier werden solche Pins gefunden und auf Wunsch als zusätzliche Öffnungen ergänzt; die
//! Regelhierarchie entscheidet dann, ob eine Öffnung entsteht ("Take Existing" → keine).

use super::types::*;
use super::StencilEngine;
use rstar::{primitives::GeomWithData, RTree};
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct MissingPin {
    pub refdes: String,
    pub pin: i64,
    pub package: String,
    pub copper: PadBox,
    pub facing: &'static str,
}

fn contains(b: &[f64; 4], x: f64, y: f64, tol: f64) -> bool {
    x >= b[0] - tol && x <= b[2] + tol && y >= b[1] - tol && y <= b[3] + tol
}

impl StencilEngine {
    pub fn missing_paste_pins(&self) -> Vec<MissingPin> {
        if self.copper_pads.is_empty() || self.odb_toeprints.is_empty() {
            return Vec::new();
        }
        let side = if self.current_side == "bot" { "BOT" } else { "TOP" };
        let comps: std::collections::HashMap<&str, &ComponentItem> =
            self.components.iter().filter(|c| c.side.eq_ignore_ascii_case(side)).map(|c| (c.refdes.as_str(), c)).collect();
        let ap_tree = RTree::bulk_load(self.apertures.iter().enumerate().map(|(i, a)| GeomWithData::new([a.cx, a.cy], i)).collect());
        let cu_tree = RTree::bulk_load(self.copper_pads.iter().enumerate().map(|(i, p)| GeomWithData::new([(p.x0 + p.x1) / 2.0, (p.y0 + p.y1) / 2.0], i)).collect());
        let mut out = Vec::new();
        for (refdes, pins) in &self.odb_toeprints {
            let Some(c) = comps.get(refdes.as_str()) else { continue };
            for &(pin, x, y) in pins {
                let covered = ap_tree.nearest_neighbor_iter(&[x, y]).take(6).any(|n| contains(&self.apertures[n.data].bbox, x, y, 0.02));
                if covered {
                    continue;
                }
                // Pinpad: unter den auf den Pin zentrierten Pads das größte (nicht die Via-Fläche darin),
                // sonst das kleinste Pad, das den Pin enthält (nicht eine Massefläche darunter)
                let cands: Vec<&PadBox> = cu_tree
                    .nearest_neighbor_iter(&[x, y])
                    .take(8)
                    .map(|n| &self.copper_pads[n.data])
                    .filter(|p| contains(&[p.x0, p.y0, p.x1, p.y1], x, y, 0.01) && p.w.max(p.h) <= 12.0)
                    .collect();
                let centered = cands.iter().filter(|p| ((p.x0 + p.x1) / 2.0 - x).hypot((p.y0 + p.y1) / 2.0 - y) <= 0.05).max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)));
                let Some(cu) = centered.or_else(|| cands.iter().min_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))).copied() else {
                    continue;
                };
                let (pcx, pcy) = ((cu.x0 + cu.x1) / 2.0, (cu.y0 + cu.y1) / 2.0);
                let (dx, dy) = (c.cx - pcx, c.cy - pcy);
                let facing = if dx.abs() < 0.05 && dy.abs() < 0.05 {
                    "center"
                } else if dx.abs() >= dy.abs() {
                    if dx > 0.0 { "right" } else { "left" }
                } else if dy > 0.0 {
                    "up"
                } else {
                    "down"
                };
                out.push(MissingPin { refdes: refdes.clone(), pin, package: c.package.clone(), copper: cu.clone(), facing });
            }
        }
        // Mehrere Toeprints auf demselben Kupferpad nur einmal
        out.sort_by(|a, b| (a.copper.x0, a.copper.y0).partial_cmp(&(b.copper.x0, b.copper.y0)).unwrap_or(std::cmp::Ordering::Equal));
        out.dedup_by(|a, b| (a.copper.x0 - b.copper.x0).abs() < 1e-6 && (a.copper.y0 - b.copper.y0).abs() < 1e-6 && (a.copper.x1 - b.copper.x1).abs() < 1e-6);
        out
    }

    pub fn missing_paste_json(&self) -> Value {
        let m = self.missing_paste_pins();
        let mut by_pkg: std::collections::BTreeMap<String, usize> = Default::default();
        for p in &m {
            *by_pkg.entry(self.resolve_alias(&p.package)).or_default() += 1;
        }
        json!({
            "count": m.len(),
            "packages": by_pkg,
            "pins": m.iter().take(200).map(|p| json!({"refdes": p.refdes, "pin": p.pin, "package": p.package, "x": p.copper.x0 / 2.0 + p.copper.x1 / 2.0, "y": p.copper.y0 / 2.0 + p.copper.y1 / 2.0, "w": p.copper.w, "h": p.copper.h})).collect::<Vec<_>>(),
        })
    }

    /// Erzeugt eine Öffnung aus einem Kupferpad (ohne Kundenpaste).
    pub(crate) fn copper_aperture(id: usize, cu: &PadBox, package: &str, refdes: &str, pin: i64, facing: &str) -> ApertureItem {
        let (cx, cy) = ((cu.x0 + cu.x1) / 2.0, (cu.y0 + cu.y1) / 2.0);
        let mut ap = ApertureItem::new(id, if cu.round { "circle" } else { "rectangle" }, cx, cy, cu.w, cu.h);
        ap.package_name = package.to_string();
        ap.component_id = refdes.to_string();
        ap.pin_number = pin;
        ap.facing_dir = facing.to_string();
        ap.copper_ref = Some(cu.clone());
        ap.from_copper = true;
        ap
    }

    /// Pins eines Packages ohne Kundenpaste (z. B. Zugentlastung an Steckern).
    pub fn missing_paste_pins_of(&self, package: &str) -> Vec<i64> {
        let target = self.resolve_alias(package);
        let mut pins: Vec<i64> = self.missing_paste_pins().into_iter().filter(|m| self.resolve_alias(&m.package) == target).map(|m| m.pin).collect();
        pins.sort_unstable();
        pins.dedup();
        pins
    }

    /// Ergänzt Öffnungen für alle Pins ohne Kundenpaste und wendet die Regeln darauf an.
    pub fn add_missing_paste(&mut self) -> usize {
        self.add_missing_paste_for(None)
    }

    /// Wie `add_missing_paste`, optional nur für ein Package.
    pub fn add_missing_paste_for(&mut self, package: Option<&str>) -> usize {
        let target = package.map(|p| self.resolve_alias(p));
        let missing: Vec<MissingPin> = self.missing_paste_pins().into_iter().filter(|m| target.as_ref().map_or(true, |t| self.resolve_alias(&m.package) == *t)).collect();
        let start = self.apertures.len();
        for m in &missing {
            let id = self.apertures.len();
            let ap = Self::copper_aperture(id, &m.copper, &m.package, &m.refdes, m.pin, m.facing);
            if self.original_apertures.len() == id {
                self.original_apertures.push(ap.clone());
            }
            self.apertures.push(ap);
        }
        let ids: Vec<usize> = (start..self.apertures.len()).collect();
        self.regenerate_ids(&ids);
        ids.len()
    }

    /// Entfernt alle ergänzten Öffnungen wieder.
    pub fn remove_copper_paste(&mut self) -> usize {
        let before = self.apertures.len();
        self.apertures.retain(|a| !a.from_copper);
        self.original_apertures.retain(|a| !a.from_copper);
        for (i, a) in self.apertures.iter_mut().enumerate() {
            a.id = i;
        }
        for (i, a) in self.original_apertures.iter_mut().enumerate() {
            a.id = i;
        }
        before - self.apertures.len()
    }
}
