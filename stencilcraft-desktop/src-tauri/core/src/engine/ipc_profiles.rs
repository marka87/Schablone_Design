//! IPC-Klassenprofile (Klasse 1 / 2 / 3).
//!
//! IPC-7525 selbst kennt keine Produktklassen; die Klassen stammen aus IPC-A-610 / J-STD-001
//! (Zuverlässigkeitsanspruch des Produkts). Ein Profil bündelt alle Schablonen-Prüfgrenzen, die
//! je Klasse strenger oder lockerer gelten sollen. Die Werte sind Vorschläge und je Kunde in der
//! Kundenbibliothek anpassbar.

use super::StencilEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcProfile {
    pub class: u8,
    pub name: String,
    /// Area Ratio (Öffnungsfläche / Wandfläche) – Mindestwert
    pub min_ar: f64,
    /// Aspect Ratio (kleinste Öffnungsbreite / Blechdicke) – Mindestwert
    pub min_aspect: f64,
    /// Warnband unterhalb der Grenzwerte in % (darunter: kritisch)
    pub warn_margin_pct: f64,
    /// Mindestanzahl Pastenkörner über die kleinste Öffnungsbreite (rechteckig / rund)
    pub balls_rect: f64,
    pub balls_round: f64,
    /// Mindest-Stegbreite im Blech (mm)
    pub min_bridge: f64,
    /// Abstand Öffnung ↔ Stufenkante in / quer zur Rakelrichtung (mm)
    pub squeegee_clearance: f64,
    pub transverse_clearance: f64,
}

pub fn default_profiles() -> BTreeMap<u8, IpcProfile> {
    let p = |class: u8, name: &str, ar: f64, asp: f64, margin: f64, br: f64, bo: f64, bridge: f64, sq: f64, tr: f64| IpcProfile {
        class,
        name: name.into(),
        min_ar: ar,
        min_aspect: asp,
        warn_margin_pct: margin,
        balls_rect: br,
        balls_round: bo,
        min_bridge: bridge,
        squeegee_clearance: sq,
        transverse_clearance: tr,
    };
    BTreeMap::from([
        (1, p(1, "Klasse 1 – Allgemeine Elektronik", 0.60, 1.40, 10.0, 4.0, 4.0, 0.08, 1.0, 0.7)),
        (2, p(2, "Klasse 2 – Erweiterte Zuverlässigkeit", 0.66, 1.50, 10.0, 5.0, 5.0, 0.10, 1.2, 0.8)),
        (3, p(3, "Klasse 3 – Hochzuverlässig", 0.70, 1.60, 15.0, 5.0, 5.0, 0.12, 1.5, 1.0)),
    ])
}

impl IpcProfile {
    /// Übernimmt bekannte Felder aus JSON (unbekannte/ungültige werden ignoriert).
    pub fn apply_map(&mut self, m: &Map<String, Value>) {
        let f = |k: &str| m.get(k).and_then(super::types::val_f64);
        if let Some(s) = m.get("name").and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty()) {
            self.name = s.trim().to_string();
        }
        if let Some(v) = f("min_ar") { self.min_ar = v.clamp(0.3, 1.5) }
        if let Some(v) = f("min_aspect") { self.min_aspect = v.clamp(0.5, 3.0) }
        if let Some(v) = f("warn_margin_pct") { self.warn_margin_pct = v.clamp(0.0, 50.0) }
        if let Some(v) = f("balls_rect") { self.balls_rect = v.clamp(1.0, 12.0) }
        if let Some(v) = f("balls_round") { self.balls_round = v.clamp(1.0, 12.0) }
        if let Some(v) = f("min_bridge") { self.min_bridge = v.clamp(0.03, 1.0) }
        if let Some(v) = f("squeegee_clearance") { self.squeegee_clearance = v.clamp(0.0, 10.0) }
        if let Some(v) = f("transverse_clearance") { self.transverse_clearance = v.clamp(0.0, 10.0) }
    }
}

impl StencilEngine {
    /// Wendet ein Klassenprofil auf alle Prüfgrenzen an.
    pub fn set_ipc_class(&mut self, class: u8) -> bool {
        let Some(p) = self.ipc_profiles.get(&class).cloned() else { return false };
        self.ipc_class = class;
        self.ipc_mode = format!("class{class}");
        self.ipc_min_ar = p.min_ar;
        self.ipc_min_aspect = p.min_aspect;
        self.ipc_warn_margin = p.warn_margin_pct / 100.0;
        self.balls_rect = p.balls_rect;
        self.balls_round = p.balls_round;
        self.min_bridge_width = p.min_bridge;
        self.squeegee_clearance = p.squeegee_clearance;
        self.transverse_clearance = p.transverse_clearance;
        true
    }

    /// Ändert ein Profil; ist es aktiv, gelten die neuen Werte sofort.
    pub fn update_ipc_profile(&mut self, class: u8, data: &Map<String, Value>) -> bool {
        let Some(p) = self.ipc_profiles.get_mut(&class) else { return false };
        p.apply_map(data);
        if self.ipc_class == class {
            self.set_ipc_class(class);
        }
        true
    }

    pub fn reset_ipc_profiles(&mut self) {
        self.ipc_profiles = default_profiles();
        if self.ipc_class > 0 {
            self.set_ipc_class(self.ipc_class);
        }
    }

    pub fn ipc_profiles_json(&self) -> Value {
        json!({
            "active_class": self.ipc_class,
            "mode": self.ipc_mode,
            "profiles": self.ipc_profiles.values().collect::<Vec<_>>(),
            "defaults": default_profiles().into_values().collect::<Vec<_>>(),
            "customer": self.current_customer,
        })
    }

    /// Bezeichnung des aktiven Prüfprofils (Bericht, Statuszeile).
    pub fn ipc_profile_label(&self) -> String {
        match self.ipc_profiles.get(&self.ipc_class).filter(|_| self.ipc_class > 0) {
            Some(p) => p.name.clone(),
            None if self.ipc_mode == "nano" => "Nano-Coated / E-Polish".into(),
            None => "Benutzerdefiniert".into(),
        }
    }

    /// Profile aus einer Kundenbibliothek übernehmen (`"ipc": {"class": n, "profiles": [...]}`).
    pub fn load_ipc_from_library(&mut self, ipc: &Value) {
        if let Some(list) = ipc.get("profiles").and_then(|v| v.as_array()) {
            for p in list {
                if let (Some(c), Some(m)) = (p.get("class").and_then(|v| v.as_u64()), p.as_object()) {
                    let entry = self.ipc_profiles.entry(c as u8).or_insert_with(|| default_profiles().get(&2).cloned().unwrap());
                    entry.class = c as u8;
                    entry.apply_map(m);
                }
            }
        }
        if let Some(c) = ipc.get("class").and_then(|v| v.as_u64()).filter(|c| (1..=3).contains(c)) {
            self.set_ipc_class(c as u8);
        }
    }

    pub fn ipc_library_json(&self) -> Value {
        json!({"class": self.ipc_class, "profiles": self.ipc_profiles.values().collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::types::ApertureItem;

    #[test]
    fn class_changes_status() {
        let mut e = StencilEngine::new(0.12, std::env::temp_dir().join("sc_ipc_test"));
        // 0.30 × 0.30 mm bei 120 µm: AR = 0.09/(1.2·0.12) = 0.625
        let ap = ApertureItem::new(0, "rectangle", 0.0, 0.0, 0.30, 0.30);
        e.set_ipc_class(1);
        assert_eq!(e.calculate_aperture_ipc(&ap).status, "OK");
        e.set_ipc_class(2);
        assert_eq!(e.calculate_aperture_ipc(&ap).status, "WARN"); // 0.625 ≥ 0.66 · 0.90
        e.set_ipc_class(3);
        // Klasse 3: AR 0.70, Warnband 15 % → 0.625 ≥ 0.595 → WARN; Aspect 2.5 ≥ 1.6
        assert_eq!(e.calculate_aperture_ipc(&ap).status, "WARN");
        assert!((e.min_bridge_width - 0.12).abs() < 1e-9);
        // Profil ändern wirkt sofort, wenn aktiv
        let m = json!({"min_ar": 0.75, "warn_margin_pct": 5}).as_object().unwrap().clone();
        e.update_ipc_profile(3, &m);
        assert_eq!(e.calculate_aperture_ipc(&ap).status, "CRIT");
        // Bibliothek: Rundreise
        let lib = e.ipc_library_json();
        let mut f = StencilEngine::new(0.12, std::env::temp_dir().join("sc_ipc_test2"));
        f.load_ipc_from_library(&lib);
        assert_eq!(f.ipc_class, 3);
        assert!((f.ipc_min_ar - 0.75).abs() < 1e-9);
    }
}
