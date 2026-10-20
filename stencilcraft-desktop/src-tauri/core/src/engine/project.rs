//! Projektdateien (.stencilproj), Undo/Redo-Verlauf, Autospeichern und "Zuletzt geöffnet".

use super::types::*;
use super::StencilEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const PROJECT_EXT: &str = "stencilproj";
const UNDO_LIMIT: usize = 80;
/// Aufeinanderfolgende Änderungen an derselben Stelle innerhalb dieser Zeit = ein Undo-Schritt
const COALESCE: Duration = Duration::from_millis(1500);

/// Bearbeitbarer Zustand (Grundlage für Undo/Redo).
#[derive(Clone)]
pub struct EditState {
    pub label: String,
    apertures: Vec<ApertureItem>,
    packages: BTreeMap<String, PackageTemplate>,
    aliases: BTreeMap<String, String>,
    zones: Vec<StencilZone>,
    base_thickness: f64,
    paste_type: String,
    ipc_mode: String,
    ipc_min_ar: f64,
    ipc_min_aspect: f64,
    squeegee_dir: String,
    current_customer: String,
    current_side: String,
    sides: BTreeMap<String, super::SideData>,
    group_rules: BTreeMap<String, PackageRule>,
    component_rules: BTreeMap<String, PackageRule>,
}

#[derive(Default)]
pub struct History {
    pub undo: Vec<EditState>,
    pub redo: Vec<EditState>,
    last_edit: Option<(String, Instant)>,
}

impl History {
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
    }
}

/// Gespeicherter Zustand einer Pastenöffnung (Zuordnung + Modifikation).
#[derive(Serialize, Deserialize)]
struct ApertureSave {
    id: usize,
    cx: f64,
    cy: f64,
    package_name: String,
    pin_number: i64,
    facing_dir: String,
    component_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    custom_slices: Option<Vec<Slice>>,
    #[serde(default)]
    manual: bool,
    /// Aus dem Kupfer ergänzte Öffnung: Kupferpad zum Wiederherstellen
    #[serde(default, skip_serializing_if = "Option::is_none")]
    copper: Option<PadBox>,
}

#[derive(Serialize, Deserialize)]
struct LayerSave {
    id: String,
    visible: bool,
    color: String,
    opacity: f64,
}

#[derive(Serialize, Deserialize)]
struct DatasetRef {
    paths: Vec<String>,
    side: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct Settings {
    base_thickness: f64,
    paste_type: String,
    ipc_mode: String,
    ipc_min_ar: f64,
    ipc_min_aspect: f64,
    squeegee_dir: String,
    min_bridge_width: f64,
    current_customer: String,
}

#[derive(Serialize, Deserialize, Default)]
struct SideSave {
    apertures: Vec<ApertureSave>,
    zones: Vec<StencilZone>,
    base_thickness: f64,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    format: String,
    version: u32,
    saved_at: String,
    dataset: DatasetRef,
    settings: Settings,
    packages: BTreeMap<String, PackageTemplate>,
    aliases: BTreeMap<String, String>,
    zones: Vec<StencilZone>,
    apertures: Vec<ApertureSave>,
    layers: Vec<LayerSave>,
    /// Version 2: Arbeitsstand je Seite (TOP/BOT)
    #[serde(default)]
    sides: BTreeMap<String, SideSave>,
    #[serde(default)]
    active_side: String,
    #[serde(default)]
    group_rules: BTreeMap<String, PackageRule>,
    #[serde(default)]
    component_rules: BTreeMap<String, PackageRule>,
}

fn aperture_saves(aps: &[ApertureItem]) -> Vec<ApertureSave> {
    aps.iter()
        .map(|a| ApertureSave {
            id: a.id,
            cx: r4(a.cx),
            cy: r4(a.cy),
            package_name: a.package_name.clone(),
            pin_number: a.pin_number,
            facing_dir: a.facing_dir.clone(),
            component_id: a.component_id.clone(),
            custom_slices: a.custom_slices.clone(),
            manual: a.is_manually_overridden,
            copper: a.copper_ref.clone().filter(|_| a.from_copper),
        })
        .collect()
}

/// Ergebnis beim Öffnen eines Projekts, dessen Datensatz nicht mehr am gespeicherten Ort liegt.
pub enum OpenResult {
    Opened { warnings: Vec<String> },
    MissingDataset { paths: Vec<String> },
}

fn now_string() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    secs.to_string()
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Ordner nicht anlegbar: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Fehler beim Schreiben von {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Fehler beim Speichern von {}: {e}", path.display()))
}

impl StencilEngine {
    // -------------------------------------------------------------
    // Undo / Redo
    // -------------------------------------------------------------
    fn snapshot(&self, label: &str) -> EditState {
        EditState {
            label: label.to_string(),
            apertures: self.apertures.clone(),
            packages: self.packages.clone(),
            aliases: self.package_aliases.clone(),
            zones: self.zones.clone(),
            base_thickness: self.base_thickness,
            paste_type: self.paste_type.clone(),
            ipc_mode: self.ipc_mode.clone(),
            ipc_min_ar: self.ipc_min_ar,
            ipc_min_aspect: self.ipc_min_aspect,
            squeegee_dir: self.squeegee_dir.clone(),
            current_customer: self.current_customer.clone(),
            current_side: self.current_side.clone(),
            sides: self.sides.clone(),
            group_rules: self.group_rules.clone(),
            component_rules: self.component_rules.clone(),
        }
    }

    fn restore(&mut self, s: EditState) {
        let side_changed = s.current_side != self.current_side;
        // Originale/Pads der wiederhergestellten Seite liegen derzeit im Seitenspeicher
        let stash = if side_changed { self.sides.get(&s.current_side).cloned() } else { None };
        self.current_side = s.current_side;
        self.sides = s.sides;
        self.group_rules = s.group_rules;
        self.component_rules = s.component_rules;
        self.apertures = s.apertures;
        self.packages = s.packages;
        self.package_aliases = s.aliases;
        self.zones = s.zones;
        self.base_thickness = s.base_thickness;
        self.paste_type = s.paste_type;
        self.ipc_mode = s.ipc_mode;
        self.ipc_min_ar = s.ipc_min_ar;
        self.ipc_min_aspect = s.ipc_min_aspect;
        self.squeegee_dir = s.squeegee_dir;
        self.current_customer = s.current_customer;
        self.bridge_violations.clear();
        if side_changed {
            // Gespeicherter Stand gehört zur anderen Seite: Originale/Pads dieser Seite übernehmen
            if let Some(d) = stash {
                self.original_apertures = d.original_apertures;
                self.pads = d.pads;
                self.copper_pads = d.copper_pads;
                self.odb_toeprints = d.odb_toeprints;
            }
            self.apply_side_visibility();
        }
    }

    /// Vor einer Bearbeitung aufrufen. `key` fasst schnelle Folgeänderungen zusammen (z. B. Spinner).
    pub fn record_undo(&mut self, label: &str, key: &str) {
        let coalesce = matches!(&self.history.last_edit, Some((k, t)) if k == key && t.elapsed() < COALESCE);
        if !coalesce {
            let snap = self.snapshot(label);
            self.history.undo.push(snap);
            if self.history.undo.len() > UNDO_LIMIT {
                self.history.undo.remove(0);
            }
        }
        self.history.redo.clear();
        self.history.last_edit = Some((key.to_string(), Instant::now()));
        self.dirty = true;
    }

    pub fn undo(&mut self) -> Option<String> {
        let prev = self.history.undo.pop()?;
        let label = prev.label.clone();
        let current = self.snapshot(&label);
        self.history.redo.push(current);
        self.restore(prev);
        self.history.last_edit = None;
        self.dirty = true;
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let next = self.history.redo.pop()?;
        let label = next.label.clone();
        let current = self.snapshot(&label);
        self.history.undo.push(current);
        self.restore(next);
        self.history.last_edit = None;
        self.dirty = true;
        Some(label)
    }

    pub fn history_json(&self) -> Value {
        json!({
            "can_undo": !self.history.undo.is_empty(),
            "can_redo": !self.history.redo.is_empty(),
            "undo_label": self.history.undo.last().map(|s| s.label.clone()),
            "redo_label": self.history.redo.last().map(|s| s.label.clone()),
            "undo_count": self.history.undo.len(),
            "dirty": self.dirty,
            "project_path": self.project_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "project_name": self.project_path.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().to_string()),
            "dataset_loaded": !self.source_paths.is_empty(),
        })
    }

    // -------------------------------------------------------------
    // Projektdatei
    // -------------------------------------------------------------
    fn project_file(&self) -> ProjectFile {
        ProjectFile {
            format: "stencilcraft-project".into(),
            version: 2,
            saved_at: now_string(),
            dataset: DatasetRef { paths: self.source_paths.clone(), side: self.source_side.clone(), name: self.dataset_name.clone() },
            settings: Settings {
                base_thickness: self.base_thickness,
                paste_type: self.paste_type.clone(),
                ipc_mode: self.ipc_mode.clone(),
                ipc_min_ar: self.ipc_min_ar,
                ipc_min_aspect: self.ipc_min_aspect,
                squeegee_dir: self.squeegee_dir.clone(),
                min_bridge_width: self.min_bridge_width,
                current_customer: self.current_customer.clone(),
            },
            packages: self.packages.clone(),
            aliases: self.package_aliases.clone(),
            zones: self.zones.clone(),
            apertures: aperture_saves(&self.apertures),
            layers: self.layers.iter().map(|l| LayerSave { id: l.id.clone(), visible: l.visible, color: l.color.clone(), opacity: l.opacity }).collect(),
            sides: {
                let mut m = BTreeMap::new();
                m.insert(self.current_side.clone(), SideSave { apertures: aperture_saves(&self.apertures), zones: self.zones.clone(), base_thickness: self.base_thickness });
                for (k, d) in &self.sides {
                    m.insert(k.clone(), SideSave { apertures: aperture_saves(&d.apertures), zones: d.zones.clone(), base_thickness: d.base_thickness });
                }
                m
            },
            active_side: self.current_side.clone(),
            group_rules: self.group_rules.clone(),
            component_rules: self.component_rules.clone(),
        }
    }

    /// Gespeicherte Öffnungen auf die aktive Seite anwenden; liefert Anzahl nicht zuordenbarer.
    fn apply_aperture_saves(&mut self, saves: Vec<ApertureSave>) -> usize {
        let mut unmatched = 0;
        // Ergänzte Kupfer-Öffnungen gibt es im neu geladenen Datensatz nicht: wieder anlegen
        let (copper, saves): (Vec<ApertureSave>, Vec<ApertureSave>) = saves.into_iter().partition(|s| s.copper.is_some());
        let had_copper = self.apertures.iter().any(|a| a.from_copper);
        for sv in copper {
            if had_copper {
                continue;
            }
            let id = self.apertures.len();
            let Some(cu) = sv.copper.clone() else { continue };
            let mut ap = Self::copper_aperture(id, &cu, &sv.package_name, &sv.component_id, sv.pin_number, &sv.facing_dir);
            ap.custom_slices = sv.custom_slices;
            ap.is_manually_overridden = sv.manual;
            if self.original_apertures.len() == id {
                self.original_apertures.push(ap.clone());
            }
            self.apertures.push(ap);
        }
        for sv in saves {
            let idx = match self.apertures.get(sv.id) {
                Some(a) if (a.cx - sv.cx).abs() < 0.01 && (a.cy - sv.cy).abs() < 0.01 => Some(sv.id),
                _ => self
                    .apertures
                    .iter()
                    .map(|a| (a.id, (a.cx - sv.cx).hypot(a.cy - sv.cy)))
                    .filter(|(_, d)| *d < 0.05)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(i, _)| i),
            };
            match idx {
                Some(i) => {
                    let a = &mut self.apertures[i];
                    a.package_name = sv.package_name;
                    a.pin_number = sv.pin_number;
                    a.facing_dir = sv.facing_dir;
                    a.component_id = sv.component_id;
                    a.custom_slices = sv.custom_slices;
                    a.is_manually_overridden = sv.manual;
                }
                None => unmatched += 1,
            }
        }
        unmatched
    }

    pub fn save_project(&mut self, path: &Path) -> Result<String, String> {
        if self.source_paths.is_empty() {
            return Err("Kein Datensatz geladen – nichts zu speichern".into());
        }
        let path = if path.extension().map_or(true, |e| e != PROJECT_EXT) { path.with_extension(PROJECT_EXT) } else { path.to_path_buf() };
        let text = serde_json::to_string(&self.project_file()).map_err(|e| e.to_string())?;
        write_atomic(&path, &text)?;
        self.project_path = Some(path.clone());
        self.dirty = false;
        let _ = std::fs::remove_file(self.autosave_path());
        self.add_recent(&path);
        Ok(path.to_string_lossy().to_string())
    }

    /// Öffnet ein Projekt: Datensatz neu importieren, danach gespeicherten Bearbeitungsstand anwenden.
    /// `dataset_override` ersetzt die gespeicherten Datensatzpfade (Datensatz wurde verschoben).
    pub fn open_project(&mut self, path: &Path, dataset_override: Option<Vec<String>>) -> Result<OpenResult, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let pf: ProjectFile = serde_json::from_str(&text).map_err(|e| format!("Ungültige Projektdatei: {e}"))?;
        if pf.format != "stencilcraft-project" {
            return Err("Keine StencilCraft-Projektdatei".into());
        }
        let paths = dataset_override.unwrap_or_else(|| pf.dataset.paths.clone());
        if paths.is_empty() || paths.iter().any(|p| !Path::new(p).exists()) {
            return Ok(OpenResult::MissingDataset { paths });
        }
        self.import_files_or_zip(&paths, &pf.dataset.side, true)?;
        let mut warnings = Vec::new();

        let s = pf.settings;
        self.base_thickness = s.base_thickness;
        self.paste_type = s.paste_type;
        self.ipc_mode = s.ipc_mode;
        self.ipc_min_ar = s.ipc_min_ar;
        self.ipc_min_aspect = s.ipc_min_aspect;
        self.squeegee_dir = s.squeegee_dir;
        self.min_bridge_width = s.min_bridge_width;
        self.current_customer = s.current_customer;
        self.packages = pf.packages;
        self.package_aliases = pf.aliases;
        if !pf.group_rules.is_empty() {
            self.group_rules = pf.group_rules;
        }
        self.component_rules = pf.component_rules;

        let mut unmatched = 0;
        if pf.sides.is_empty() {
            // Version 1: nur eine Seite
            self.zones = pf.zones;
            unmatched += self.apply_aperture_saves(pf.apertures);
        } else {
            for (side, sv) in pf.sides {
                self.switch_side(&side);
                self.zones = sv.zones;
                if sv.base_thickness > 0.0 {
                    self.base_thickness = sv.base_thickness;
                }
                unmatched += self.apply_aperture_saves(sv.apertures);
            }
            let active = if pf.active_side.is_empty() { pf.dataset.side.clone() } else { pf.active_side.clone() };
            self.switch_side(&active);
        }
        if unmatched > 0 {
            warnings.push(format!("{unmatched} gespeicherte Öffnungen passen nicht mehr zum Datensatz (wurde er geändert?)"));
        }
        for ls in pf.layers {
            if let Some(l) = self.layer_mut(&ls.id) {
                l.visible = ls.visible;
                l.color = ls.color;
                l.opacity = ls.opacity;
            }
        }
        if !pf.dataset.name.is_empty() {
            self.dataset_name = pf.dataset.name;
        }
        self.history.clear();
        self.dirty = false;
        self.project_path = Some(path.to_path_buf());
        if !path.starts_with(self.data_root.join("Autosave")) {
            self.add_recent(path);
        }
        Ok(OpenResult::Opened { warnings })
    }

    // -------------------------------------------------------------
    // Autospeichern & zuletzt geöffnet
    // -------------------------------------------------------------
    /// Sicherungsdatei dieser Instanz (je Prozess eigene Datei, damit parallele Instanzen sich nicht überschreiben).
    pub fn autosave_path(&self) -> PathBuf {
        self.data_root.join("Autosave").join(format!("autosave_{}.{PROJECT_EXT}", std::process::id()))
    }

    /// Neueste Sicherung einer anderen (beendeten oder abgestürzten) Instanz.
    fn foreign_autosave(&self) -> Option<PathBuf> {
        let own = self.autosave_path();
        std::fs::read_dir(self.data_root.join("Autosave"))
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p != &own && p.extension().map_or(false, |x| x == PROJECT_EXT))
            .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
    }

    /// Schreibt bei ungespeicherten Änderungen eine Sicherung (Wiederherstellung nach Absturz).
    pub fn autosave(&self) -> Result<bool, String> {
        if !self.dirty || self.source_paths.is_empty() {
            return Ok(false);
        }
        let mut pf = self.project_file();
        pf.dataset.name = format!("{} (wiederhergestellt)", pf.dataset.name);
        let text = serde_json::to_string(&pf).map_err(|e| e.to_string())?;
        write_atomic(&self.autosave_path(), &text)?;
        Ok(true)
    }

    /// Eigene Sicherung löschen; optional zusätzlich eine angebotene fremde Sicherung.
    pub fn discard_autosave(&self, other: Option<&Path>) {
        let _ = std::fs::remove_file(self.autosave_path());
        if let Some(o) = other.filter(|o| o.starts_with(self.data_root.join("Autosave"))) {
            let _ = std::fs::remove_file(o);
        }
    }

    pub fn recovery_info(&self) -> Value {
        let Some(p) = self.foreign_autosave() else { return json!({"exists": false}) };
        match std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            Some(v) => json!({
                "exists": true,
                "path": p.to_string_lossy(),
                "dataset": v["dataset"]["name"],
                "saved_at": v["saved_at"],
            }),
            None => json!({"exists": false}),
        }
    }

    fn recent_file(&self) -> PathBuf {
        self.data_root.join("recent_projects.json")
    }

    pub fn recent_projects(&self) -> Vec<String> {
        std::fs::read_to_string(self.recent_file())
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|p| Path::new(p).exists())
            .collect()
    }

    fn add_recent(&self, path: &Path) {
        let p = path.to_string_lossy().to_string();
        let mut list = self.recent_projects();
        list.retain(|x| !x.eq_ignore_ascii_case(&p));
        list.insert(0, p);
        list.truncate(10);
        if let Ok(t) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(self.recent_file(), t);
        }
    }
}

/// Bezeichnung einer Bearbeitung für die Undo-Anzeige; `None` = Route ändert nichts Rückgängig-machbares.
pub fn edit_label(method: &str, seg: &[&str], body: &Map<String, Value>) -> Option<String> {
    let id = || body.get("id").or_else(|| body.get("ap_id")).map(|v| v.to_string()).unwrap_or_default();
    Some(match (method, seg) {
        ("POST", ["api", "slice"]) => format!("Slicing Pad {}", id()),
        ("POST", ["api", "reduce"]) => format!("Flächenreduktion Pad {}", id()),
        ("POST", ["api", "elongate"]) => format!("Länglich & Schmal Pad {}", id()),
        ("POST", ["api", "reset_aperture"]) => format!("Zurücksetzen Pad {}", id()),
        ("POST", ["api", "pip", "apply"]) => format!("Pin-in-Paste Pad {}", id()),
        ("POST", ["api", "pad", p, "override"]) => format!("Pad {p} anpassen"),
        ("POST", ["api", "pad", p, "reset"]) => format!("Pad {p} zurücksetzen"),
        ("POST", ["api", "zone"]) => "Zone anlegen".into(),
        ("POST", ["api", "zone", "around_component"]) => "Stufenzone um Bauteil".into(),
        ("PUT" | "POST", ["api", "zone", z]) => format!("Zone {z} ändern"),
        ("DELETE", ["api", "zone", z]) => format!("Zone {z} löschen"),
        ("POST", ["api", "pcbi", "import"]) => "PCB-I-Kundenbibliothek importiert".into(),
        ("POST", ["api", "paste", "add_missing"]) => "Fehlende Pastenöffnungen ergänzt".into(),
        ("POST", ["api", "paste", "remove_added"]) => "Ergänzte Pastenöffnungen entfernt".into(),
        ("POST", ["api", "package", n, "rule"]) => format!("Regel {n}"),
        ("POST", ["api", "package", n, "generate"]) => format!("Regel anwenden {n}"),
        ("POST", ["api", "package", n, "rename"]) => format!("Umbenennen {n}"),
        ("POST" | "DELETE", ["api", "package", n, "pin", p, "override"]) => format!("Pin {p} von {n}"),
        ("POST" | "DELETE", ["api", "package", n, "pins", "override"]) => format!("Alle Pins von {n}"),
        ("POST", ["api", "auto_classify"]) => "Auto-Klassifizierung".into(),
        ("POST", ["api", "group", g, "rule"]) => format!("Gruppenregel {g}"),
        ("POST", ["api", "group", g, "generate"]) => format!("Gruppe {g} anwenden"),
        ("POST", ["api", "component", c, "rule"]) => format!("Bauteilregel {c}"),
        ("POST", ["api", "component", c, "generate"]) => format!("Bauteil {c} anwenden"),
        ("POST", ["api", "zone", z, "rule"]) => format!("Zonenregel {z}"),
        ("POST", ["api", "zone", z, "generate"]) => format!("Zone {z} anwenden"),
        ("POST", ["api", "generate_all"]) => "Alle Regeln anwenden".into(),
        ("POST", ["api", "library", "load"]) => "Bibliothek laden".into(),
        ("POST", ["api", "set_ipc_mode"]) => "IPC-Klasse".into(),
        ("POST", ["api", "set_paste_type"]) => "Pastentyp".into(),
        ("POST", ["api", "set_squeegee"]) => "Rakelrichtung".into(),
        ("POST", ["api", "set_thickness"]) => "Blechdicke".into(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::handle;

    fn engine(name: &str) -> (StencilEngine, PathBuf) {
        let root = std::env::temp_dir().join(format!("stencilcraft_project_test_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut e = StencilEngine::new(0.12, root.clone());
        let mut g = String::from("%FSLAX46Y46*%%MOMM*%%ADD10R,1.2X1.3*%D10*\n");
        for i in 0..3 {
            let x = 10.0 + i as f64 * 5.0;
            g.push_str(&format!("X{}Y10000000D03*\nX{}Y10000000D03*\n", ((x - 0.851) * 1e6) as i64, ((x + 0.851) * 1e6) as i64));
        }
        g.push_str("M02*");
        let gp = root.join("proj_top_paste.ger");
        std::fs::write(&gp, g).unwrap();
        e.import_files_or_zip(&[gp.to_string_lossy().to_string()], "top", true).unwrap();
        (e, root)
    }

    #[test]
    fn undo_redo_and_coalescing() {
        let (mut e, _) = engine("undo");
        let empty = Map::new();
        let rule = |cut: f64| json!({"rule_type": "house_builder", "house_cut_pct": cut});
        handle(&mut e, "POST", "/api/package/0805/generate", rule(30.0).as_object().unwrap(), &empty);
        // zwei schnelle Änderungen derselben Regel → ein Undo-Schritt
        handle(&mut e, "POST", "/api/package/0805/rule", rule(40.0).as_object().unwrap(), &empty);
        handle(&mut e, "POST", "/api/package/0805/rule", rule(50.0).as_object().unwrap(), &empty);
        assert_eq!(e.history.undo.len(), 2);
        assert_eq!(e.packages["0805"].rule.house_cut_pct, 50.0);
        assert!(e.undo().is_some());
        assert_eq!(e.packages["0805"].rule.house_cut_pct, 30.0);
        assert!(e.apertures[0].custom_slices.is_some());
        assert!(e.undo().is_some());
        assert!(e.apertures[0].custom_slices.is_none());
        assert!(e.redo().is_some());
        assert!(e.apertures[0].custom_slices.is_some());
    }

    #[test]
    fn project_roundtrip_restores_edits() {
        let (mut e, root) = engine("roundtrip");
        let empty = Map::new();
        handle(&mut e, "POST", "/api/package/0805/generate", json!({"rule_type": "house_builder"}).as_object().unwrap(), &empty);
        handle(&mut e, "POST", "/api/zone", json!({"x0": 1, "y0": 1, "x1": 20, "y1": 20, "thickness": 0.1}).as_object().unwrap(), &empty);
        handle(&mut e, "POST", "/api/package/0805/rename", json!({"new_name": "CHIP_0805_KUNDE"}).as_object().unwrap(), &empty);
        assert!(e.dirty);
        let pp = root.join("Test.stencilproj");
        e.save_project(&pp).unwrap();
        assert!(!e.dirty);

        let mut f = StencilEngine::new(0.12, root.clone());
        match f.open_project(&pp, None).unwrap() {
            OpenResult::Opened { warnings } => assert!(warnings.is_empty(), "{warnings:?}"),
            OpenResult::MissingDataset { .. } => panic!("Datensatz fehlt"),
        }
        assert_eq!(f.zones.len(), 1);
        assert_eq!(f.apertures[0].package_name, "CHIP_0805_KUNDE");
        assert!(f.packages.contains_key("CHIP_0805_KUNDE"));
        assert!(f.apertures[0].custom_slices.as_ref().unwrap()[0].pts.is_some());
        assert_eq!(f.recent_projects().len(), 1);
    }

    #[test]
    fn missing_dataset_is_reported() {
        let (mut e, root) = engine("missing");
        let pp = root.join("Moved.stencilproj");
        e.save_project(&pp).unwrap();
        let text = std::fs::read_to_string(&pp).unwrap().replace("proj_top_paste.ger", "verschoben.ger");
        std::fs::write(&pp, text).unwrap();
        let mut f = StencilEngine::new(0.12, root);
        assert!(matches!(f.open_project(&pp, None).unwrap(), OpenResult::MissingDataset { .. }));
    }
}
