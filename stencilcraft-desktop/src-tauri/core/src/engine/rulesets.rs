//! Regelhierarchie wie im PCB Investigator:
//! Bauteilregel (RefDes) → Zonenregel → Package-Regel (+ Pin-Anpassung) → Gruppenregel ("Inherit").

use super::rules::{generate_rule_aperture_geometry, take_existing_rounded, PadCtx};
use super::types::*;
use super::StencilEngine;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const GROUPS: [(&str, &str); 5] = [
    ("SMD", "SMD Packages"),
    ("THT", "THT Packages"),
    ("Other", "Other Packages"),
    ("Fiducial", "Fiducials"),
    ("Library", "Bibliothek"),
];

pub fn group_label(key: &str) -> &'static str {
    GROUPS.iter().find(|(k, _)| *k == key).map_or("SMD Packages", |(_, l)| *l)
}

/// Standard-Gruppenregeln: SMD automatisch je Pad, THT/Other/Fiducials Kundenpaste übernehmen.
pub fn default_group_rules() -> BTreeMap<String, PackageRule> {
    let pg = PackageRule::with("package_group", "Handle Stencil Creation by Package Group");
    let keep = PackageRule::with("take_existing", "Take Existing Paste Elements");
    BTreeMap::from([
        ("SMD".to_string(), pg.clone()),
        ("THT".to_string(), keep.clone()),
        ("Other".to_string(), keep.clone()),
        ("Fiducial".to_string(), keep),
        ("Library".to_string(), pg),
    ])
}

impl StencilEngine {
    pub fn group_rule(&self, group: &str) -> PackageRule {
        self.group_rules.get(group).cloned().unwrap_or_else(|| self.group_rules.get("SMD").cloned().unwrap_or_default())
    }

    /// Regel eines Packages, "inherit" aufgelöst über die Gruppe.
    pub fn package_effective_rule(&self, pkg: &PackageTemplate) -> (PackageRule, String) {
        if pkg.rule.rule_type == "inherit" {
            (self.group_rule(&pkg.group), format!("Gruppe {}", group_label(&pkg.group)))
        } else {
            (pkg.rule.clone(), format!("Package {}", pkg.name))
        }
    }

    /// Wirksame Regel einer Öffnung und ihre Herkunft.
    pub fn effective_rule(&self, ap: &ApertureItem) -> (PackageRule, String) {
        if let Some(r) = self.component_rules.get(&ap.component_id).filter(|_| !ap.component_id.is_empty()) {
            return (r.clone(), format!("Bauteil {}", ap.component_id));
        }
        if let Some(z) = self.zones.iter().find(|z| z.rule.is_some() && z.contains_point(ap.cx, ap.cy)) {
            return (z.rule.clone().unwrap_or_default(), format!("Zone {}", z.name));
        }
        match self.packages.get(&self.resolve_alias(&ap.package_name)) {
            Some(pkg) => self.package_effective_rule(pkg),
            None => (PackageRule::with("take_existing", "Take Existing"), "keine Regel".into()),
        }
    }

    /// Bohrdurchmesser an einer Position (THT), falls eine Bohrung in Reichweite liegt.
    pub fn drill_at(&self, x: f64, y: f64, size: f64) -> Option<f64> {
        let tol = (size / 2.0).max(0.3);
        self.drills
            .iter()
            .map(|d| (d.2, (d.0 - x).hypot(d.1 - y)))
            .filter(|(_, dist)| *dist <= tol)
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(d, _)| d)
    }

    /// Öffnungen neu berechnen (manuell bearbeitete bleiben unverändert).
    pub fn regenerate_ids(&mut self, ids: &[usize]) -> usize {
        let ids: Vec<usize> = ids.iter().copied().filter(|&i| i < self.apertures.len() && !self.apertures[i].is_manually_overridden).collect();
        let pitches = self.aperture_pitches(&ids);
        let mut updates = Vec::with_capacity(ids.len());
        for &id in &ids {
            let ap = &self.apertures[id];
            let slices = match self.packages.get(&self.resolve_alias(&ap.package_name)) {
                Some(pkg) => self.regenerate_from_package(ap, pkg, pitches.get(&id).copied()),
                None => {
                    let (rule, _) = self.effective_rule(ap);
                    let ctx = PadCtx {
                        round: ap.shape_type == "circle",
                        min_bridge: self.min_bridge_width,
                        thickness: self.get_effective_thickness(ap.cx, ap.cy).0,
                        drill: self.drill_at(ap.cx, ap.cy, ap.width.min(ap.height)),
                        ..PadCtx::new(ap.width, ap.height, ap.cx, ap.cy, &ap.facing_dir)
                    };
                    generate_rule_aperture_geometry(&ctx, &rule).or_else(|| take_existing_rounded(&rule, ap.shape_type == "rectangle" && !ap.from_copper, ap.cx, ap.cy, ap.width, ap.height))
                }
            };
            // Ergänzte Kupfer-Öffnung ohne erzeugende Regel ("Take Existing"): keine Paste
            let slices = if ap.from_copper && slices.is_none() { Some(Vec::new()) } else { slices };
            updates.push((id, slices));
        }
        let n = updates.len();
        for (id, s) in updates {
            self.apertures[id].custom_slices = s;
        }
        n
    }

    /// Alle Regeln auf alle (nicht manuellen) Öffnungen der aktiven Seite anwenden.
    pub fn generate_all(&mut self) -> usize {
        let ids: Vec<usize> = (0..self.apertures.len()).collect();
        self.regenerate_ids(&ids)
    }

    // -------------------------------------------------------------
    // Gruppenregeln
    // -------------------------------------------------------------
    pub fn set_group_rule(&mut self, group: &str, data: &Map<String, Value>) {
        let mut r = self.group_rule(group);
        r.apply_map(data);
        if r.rule_type == "inherit" {
            r.rule_type = "package_group".into(); // Gruppen erben nicht weiter
        }
        self.group_rules.insert(group.to_string(), r);
    }

    /// Öffnungen, deren Package die Regel von `group` erbt (ohne Bauteil-/Zonenregel).
    pub fn group_aperture_ids(&self, group: &str) -> Vec<usize> {
        self.apertures
            .iter()
            .filter(|a| {
                let src = self.effective_rule(a).1;
                src == format!("Gruppe {}", group_label(group))
            })
            .map(|a| a.id)
            .collect()
    }

    pub fn groups_json(&self) -> Value {
        let list: Vec<Value> = GROUPS
            .iter()
            .map(|(k, label)| {
                let pkgs = self.packages.values().filter(|p| p.group == *k).count();
                let inheriting = self.packages.values().filter(|p| p.group == *k && p.rule.rule_type == "inherit").count();
                json!({"key": k, "label": label, "rule": self.group_rule(k).to_json(), "packages": pkgs, "inheriting": inheriting, "pads": self.group_aperture_ids(k).len()})
            })
            .collect();
        json!({"groups": list})
    }

    // -------------------------------------------------------------
    // Bauteilregeln
    // -------------------------------------------------------------
    pub fn component_aperture_ids(&self, refdes: &str) -> Vec<usize> {
        self.apertures.iter().filter(|a| a.component_id == refdes).map(|a| a.id).collect()
    }

    /// Setzt/ergänzt die Bauteilregel (Basis: bisher wirksame Regel) oder entfernt sie (`enabled: false`).
    pub fn set_component_rule(&mut self, refdes: &str, data: &Map<String, Value>) {
        if data.get("enabled").and_then(val_bool) == Some(false) {
            self.component_rules.remove(refdes);
            return;
        }
        let mut r = match self.component_rules.get(refdes) {
            Some(r) => r.clone(),
            None => self.apertures.iter().find(|a| a.component_id == refdes).map(|a| self.effective_rule(a).0).unwrap_or_default(),
        };
        let mut d = data.clone();
        d.remove("enabled");
        r.apply_map(&d);
        if r.rule_type == "inherit" {
            // "Inherit" auf Bauteilebene = Bauteilregel entfernen
            self.component_rules.remove(refdes);
            return;
        }
        self.component_rules.insert(refdes.to_string(), r);
    }

    pub fn components_json(&self) -> Value {
        let mut pads: BTreeMap<&str, usize> = BTreeMap::new();
        for a in &self.apertures {
            *pads.entry(a.component_id.as_str()).or_default() += 1;
        }
        let list: Vec<Value> = self
            .components
            .iter()
            .filter(|c| c.side.eq_ignore_ascii_case(&self.current_side))
            .map(|c| {
                json!({
                    "refdes": c.refdes,
                    "package": c.package,
                    "rot": c.rotation,
                    "pads": pads.get(c.refdes.as_str()).copied().unwrap_or(0),
                    "has_rule": self.component_rules.contains_key(&c.refdes),
                })
            })
            .collect();
        json!({"components": list})
    }

    pub fn component_json(&self, refdes: &str) -> Value {
        let comp = self.components.iter().find(|c| c.refdes == refdes);
        let pkg = comp.map(|c| c.package.clone()).or_else(|| self.apertures.iter().find(|a| a.component_id == refdes).map(|a| a.package_name.clone())).unwrap_or_default();
        let own = self.component_rules.get(refdes);
        let effective = match own {
            Some(r) => (r.clone(), format!("Bauteil {refdes}")),
            None => self.apertures.iter().find(|a| a.component_id == refdes).map(|a| self.effective_rule(a)).unwrap_or_else(|| (PackageRule::default(), "keine Regel".into())),
        };
        let preview = self.package_preview_with(&pkg, Some((&effective.0, effective.1.clone())));
        json!({
            "refdes": refdes,
            "package": pkg,
            "rot": comp.map(|c| c.rotation),
            "has_rule": own.is_some(),
            "rule": own.map(|r| r.to_json()),
            "effective_rule": effective.0.to_json(),
            "rule_source": effective.1,
            "preview": preview,
            "pads": self.component_aperture_ids(refdes).len(),
        })
    }

    // -------------------------------------------------------------
    // Zonenregeln
    // -------------------------------------------------------------
    pub fn zone_aperture_ids(&self, zone_id: &str) -> Vec<usize> {
        match self.zones.iter().find(|z| z.id == zone_id) {
            Some(z) => self.apertures.iter().filter(|a| z.contains_point(a.cx, a.cy)).map(|a| a.id).collect(),
            None => Vec::new(),
        }
    }

    pub fn set_zone_rule(&mut self, zone_id: &str, data: &Map<String, Value>) -> bool {
        let disable = data.get("enabled").and_then(val_bool) == Some(false);
        let base = self.group_rule("SMD");
        let Some(z) = self.zones.iter_mut().find(|z| z.id == zone_id) else { return false };
        if disable {
            z.rule = None;
            return true;
        }
        let mut r = z.rule.clone().unwrap_or(base);
        let mut d = data.clone();
        d.remove("enabled");
        r.apply_map(&d);
        if r.rule_type == "inherit" {
            z.rule = None;
        } else {
            z.rule = Some(r);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::api::handle;
    use crate::engine::StencilEngine;
    use serde_json::{json, Map};

    #[test]
    fn rule_precedence_component_zone_package_group() {
        let root = std::env::temp_dir().join("stencilcraft_rules_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut g = String::from("%FSLAX46Y46*%%MOMM*%%ADD10R,1.2X1.3*%D10*\n");
        for i in 0..4 {
            let x = 10.0 + i as f64 * 6.0;
            g.push_str(&format!("X{}Y10000000D03*\nX{}Y10000000D03*\n", ((x - 0.851) * 1e6) as i64, ((x + 0.851) * 1e6) as i64));
        }
        g.push_str("M02*");
        let gp = root.join("r_top_paste.ger");
        std::fs::write(&gp, g).unwrap();
        let mut e = StencilEngine::new(0.12, root.clone());
        e.import_files_or_zip(&[gp.to_string_lossy().to_string()], "top", true).unwrap();
        let empty = Map::new();
        // Package erbt von der Gruppe
        handle(&mut e, "POST", "/api/package/0805/rule", json!({"rule_type": "inherit"}).as_object().unwrap(), &empty);
        handle(&mut e, "POST", "/api/group/SMD/rule", json!({"rule_type": "inverted_house_builder"}).as_object().unwrap(), &empty);
        let (_, v) = handle(&mut e, "POST", "/api/group/SMD/generate", &empty, &empty);
        assert_eq!(v["modified_count"], 8, "{v}");
        assert_eq!(e.effective_rule(&e.apertures[0]).1, "Gruppe SMD Packages");
        assert_eq!(e.apertures[0].custom_slices.as_ref().unwrap()[0].kind.as_deref(), Some("inverted_house_builder"));
        // Zonenregel über Bauteil 2 (x ≈ 16)
        let (_, z) = handle(&mut e, "POST", "/api/zone", json!({"x0": 14, "y0": 8, "x1": 18, "y1": 12, "step": false}).as_object().unwrap(), &empty);
        let zid = z["zone"]["id"].as_str().unwrap().to_string();
        assert_eq!(z["zone"]["step"], false);
        handle(&mut e, "POST", &format!("/api/zone/{zid}/rule"), json!({"rule_type": "simple_rect", "corner_radius": 0.0}).as_object().unwrap(), &empty);
        handle(&mut e, "POST", &format!("/api/zone/{zid}/generate"), &empty, &empty);
        let in_zone = e.apertures.iter().find(|a| (a.cx - 16.0).abs() < 1.0).unwrap();
        assert!(e.effective_rule(in_zone).1.starts_with("Zone"));
        assert_eq!(e.get_effective_thickness(in_zone.cx, in_zone.cy).0, 0.12, "Regelzone ändert keine Dicke");
        // Bauteilregel schlägt Zone
        let refdes = in_zone.component_id.clone();
        handle(&mut e, "POST", &format!("/api/component/{refdes}/rule"), json!({"rule_type": "house_builder"}).as_object().unwrap(), &empty);
        handle(&mut e, "POST", &format!("/api/component/{refdes}/generate"), &empty, &empty);
        let ap = e.apertures.iter().find(|a| a.component_id == refdes).unwrap();
        assert_eq!(e.effective_rule(ap).1, format!("Bauteil {refdes}"));
        assert_eq!(ap.custom_slices.as_ref().unwrap()[0].kind.as_deref(), Some("house_builder"));
        // Undo stellt den vorherigen Stand wieder her
        assert!(e.undo().is_some());
        assert!(e.undo().is_some());
        assert!(!e.component_rules.contains_key(&refdes));
    }
}
