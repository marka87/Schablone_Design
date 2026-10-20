//! Stufenzonen vorschlagen: um alle Bauteile eines Packages (oder alle begrenzenden Packages),
//! nahe beieinander liegende Bauteile zu einer gemeinsamen Zone zusammengefasst.

use super::dfm::{recommended_step, STD_THICKNESSES};
use super::StencilEngine;
use serde_json::{json, Value};

/// Rechteck [x0, y0, x1, y1]
type Rect = [f64; 4];

fn overlaps(a: &Rect, b: &Rect, gap: f64) -> bool {
    a[0] - gap <= b[2] && b[0] - gap <= a[2] && a[1] - gap <= b[3] && b[1] - gap <= a[3]
}

fn union(a: &Rect, b: &Rect) -> Rect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Fasst Rechtecke zusammen, die sich überlappen oder näher als `gap` liegen (bis nichts mehr passt).
pub fn merge_rects(mut rects: Vec<(Rect, Vec<String>)>, gap: f64) -> Vec<(Rect, Vec<String>)> {
    loop {
        let mut merged = false;
        'outer: for i in 0..rects.len() {
            for j in (i + 1)..rects.len() {
                if overlaps(&rects[i].0, &rects[j].0, gap) {
                    let (b, comps) = rects.remove(j);
                    rects[i].0 = union(&rects[i].0, &b);
                    rects[i].1.extend(comps);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            return rects;
        }
    }
}

impl StencilEngine {
    /// Rakel- (in Rakelrichtung) und Querabstand als Rand um die Öffnungen.
    fn zone_margins(&self) -> (f64, f64) {
        let (along, across) = (self.squeegee_clearance + 0.05, self.transverse_clearance + 0.05);
        if self.squeegee_dir == "Y" { (across, along) } else { (along, across) }
    }

    /// Zonenvorschläge um alle Bauteile eines Packages auf der aktiven Seite.
    pub fn propose_zones_for_package(&self, package: &str, merge_gap: f64) -> Vec<Value> {
        let (mx, my) = self.zone_margins();
        let target = self.resolve_alias(package);
        let mut per_comp: std::collections::BTreeMap<String, Rect> = Default::default();
        for a in self.apertures.iter().filter(|a| self.resolve_alias(&a.package_name) == target) {
            let key = if a.component_id.is_empty() { format!("#{}", a.id) } else { a.component_id.clone() };
            let r = per_comp.entry(key).or_insert([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]);
            *r = [r[0].min(a.bbox[0]), r[1].min(a.bbox[1]), r[2].max(a.bbox[2]), r[3].max(a.bbox[3])];
        }
        let rects: Vec<(Rect, Vec<String>)> = per_comp.into_iter().map(|(c, r)| ([r[0] - mx, r[1] - my, r[2] + mx, r[3] + my], vec![c])).collect();
        merge_rects(rects, merge_gap.max(0.0))
            .into_iter()
            .map(|(r, comps)| {
                // Fremde Öffnungen in der Zone bekommen ebenfalls die dünnere Stufe
                let inside: Vec<&super::types::ApertureItem> = self.apertures.iter().filter(|a| a.cx >= r[0] && a.cx <= r[2] && a.cy >= r[1] && a.cy <= r[3]).collect();
                let foreign = inside.iter().filter(|a| self.resolve_alias(&a.package_name) != target).count();
                let overlaps_zone = self.zones.iter().any(|z| z.step && overlaps(&r, &[z.x0, z.y0, z.x1, z.y1], 0.0));
                json!({
                    "x0": r[0], "y0": r[1], "x1": r[2], "y1": r[3],
                    "components": comps, "pads": inside.len() - foreign, "foreign_pads": foreign,
                    "overlaps_zone": overlaps_zone,
                })
            })
            .collect()
    }

    /// Kleinste zulässige Dicke über alle Öffnungen eines Packages → Standardschritt.
    pub fn package_step(&self, package: &str) -> f64 {
        let target = self.resolve_alias(package);
        let t = self.apertures.iter().filter(|a| self.resolve_alias(&a.package_name) == target).map(|a| self.max_thickness_for(a)).fold(f64::INFINITY, f64::min);
        if t.is_finite() { recommended_step(t) } else { self.base_thickness }
    }

    /// Automatische Vorschläge: alle Packages, die bei der Grunddicke nicht ausreichen.
    pub fn suggest_zones(&self, merge_gap: f64) -> Vec<Value> {
        let mut pkgs: std::collections::BTreeMap<String, f64> = Default::default();
        for a in &self.apertures {
            if self.get_effective_thickness(a.cx, a.cy).1.is_some() {
                continue; // liegt bereits in einer Stufenzone
            }
            let tm = self.max_thickness_for(a);
            if tm < self.base_thickness - 1e-9 && !a.package_name.is_empty() {
                let e = pkgs.entry(self.resolve_alias(&a.package_name)).or_insert(f64::INFINITY);
                *e = e.min(tm);
            }
        }
        pkgs.into_iter()
            .map(|(p, tm)| {
                let step = recommended_step(tm).min(*STD_THICKNESSES.last().unwrap());
                json!({"package": p, "t_max": tm, "thickness": step, "zones": self.propose_zones_for_package(&p, merge_gap)})
            })
            .collect()
    }

    /// Legt Zonen aus Rechtecken an (ein Undo-Schritt über die API).
    pub fn create_zones(&mut self, rects: &[Rect], thickness: f64, name: &str) -> usize {
        for (i, r) in rects.iter().enumerate() {
            let label = if rects.len() > 1 { format!("{name} {}", i + 1) } else { name.to_string() };
            self.add_zone_ext(r[0], r[1], r[2], r[3], thickness, &label, true);
        }
        rects.len()
    }

    /// Lage/Größe einer Zone ändern.
    pub fn set_zone_geometry(&mut self, id: &str, r: Rect) -> bool {
        let Some(z) = self.zones.iter_mut().find(|z| z.id == id) else { return false };
        z.x0 = r[0].min(r[2]);
        z.y0 = r[1].min(r[3]);
        z.x1 = r[0].max(r[2]);
        z.y1 = r[1].max(r[3]);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_near_rects() {
        let r = vec![([0.0, 0.0, 1.0, 1.0], vec!["A".into()]), ([1.5, 0.0, 2.5, 1.0], vec!["B".into()]), ([10.0, 0.0, 11.0, 1.0], vec!["C".into()])];
        let m = merge_rects(r.clone(), 1.0);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].1, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(merge_rects(r, 0.0).len(), 3);
    }
}
