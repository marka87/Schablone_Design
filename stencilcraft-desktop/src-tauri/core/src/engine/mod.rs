//! SMT Stencil Design & Verification Engine (native Rust-Portierung der Web-Engine `stencil_engine.py`).
//!
//! Arbeitsweise strikt non-destruktiv: `original_apertures` bleibt unverändert (Kundenlage),
//! alle Modifikationen landen als `custom_slices` auf den Arbeits-Aperturen (Zwischenspeicher).

pub mod copper_paste;
pub mod dfm;
pub mod export;
pub mod geometry;
pub mod gerber;
pub mod import;
pub mod odb;
pub mod packages;
pub mod pcbi;
pub mod project;
pub mod report;
pub mod rules;
pub mod rulesets;
pub mod types;

use geometry::circle_pts;
use rules::{elongated_narrow_width, generate_rule_aperture_geometry, resolve_pin_rule, PadCtx};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use types::*;

pub struct StencilEngine {
    pub base_thickness: f64,
    pub ipc_min_ar: f64,
    pub ipc_min_aspect: f64,
    pub ipc_mode: String,
    pub squeegee_dir: String,
    pub squeegee_clearance: f64,
    pub transverse_clearance: f64,

    pub current_side: String,
    pub dataset_name: String,
    pub original_apertures: Vec<ApertureItem>,
    pub apertures: Vec<ApertureItem>,
    pub pads: Vec<PadBox>,
    /// Kupferpads der Außenlage der aktiven Seite (ODB++)
    pub copper_pads: Vec<PadBox>,
    pub components: Vec<ComponentItem>,
    pub packages: BTreeMap<String, PackageTemplate>,
    pub package_aliases: BTreeMap<String, String>,
    /// `<Settings>`-Block der zuletzt importierten PCB-I-Kundenbibliothek
    pub pcbi_settings_xml: Option<String>,
    pub current_customer: String,
    pub zones: Vec<StencilZone>,
    pub profile_paths: Vec<[Pt; 2]>,
    pub fiducials: Vec<(f64, f64, f64)>,
    pub layers: Vec<BoardLayer>,
    pub bounds: [f64; 4],

    pub paste_type: String,
    pub min_bridge_width: f64,
    pub bridge_violations: Vec<BridgeViolation>,
    /// ODB++ Toeprints (RefDes, [(Pin, x, y)]) der aktiven Seite für die Pin-Zuordnung
    pub odb_toeprints: Vec<(String, Vec<(i64, f64, f64)>)>,
    /// Bohrungen (x, y, Durchmesser) aus den Bohrlagen
    pub drills: Vec<(f64, f64, f64)>,

    /// Quelle des geladenen Datensatzes (für Projektdateien)
    pub source_paths: Vec<String>,
    pub source_side: String,
    /// Pfad der geöffneten Projektdatei
    pub project_path: Option<PathBuf>,
    /// Ungespeicherte Änderungen vorhanden
    pub dirty: bool,
    pub history: project::History,
    /// Arbeitsstand der nicht aktiven Seite(n)
    pub sides: BTreeMap<String, SideData>,
    /// Gruppenregeln (SMD / THT / Other / Fiducial / Library) – Ziel von "Inherit"
    pub group_rules: BTreeMap<String, PackageRule>,
    /// Bauteilregeln je RefDes (höchste Priorität)
    pub component_rules: BTreeMap<String, PackageRule>,

    pub data_root: PathBuf,
}

pub const NO_DATASET: &str = "Kein Datensatz geladen";

/// Seitenspezifischer Arbeitsstand (für die jeweils nicht aktive Seite TOP/BOT).
#[derive(Debug, Clone, Default)]
pub struct SideData {
    pub apertures: Vec<ApertureItem>,
    pub original_apertures: Vec<ApertureItem>,
    pub zones: Vec<StencilZone>,
    pub pads: Vec<PadBox>,
    pub copper_pads: Vec<PadBox>,
    pub odb_toeprints: Vec<(String, Vec<(i64, f64, f64)>)>,
    pub bridge_violations: Vec<BridgeViolation>,
    pub base_thickness: f64,
}

/// Ermittelt das Datenverzeichnis (enthält `Libraries/` und `Export/`).
/// Reihenfolge: Umgebungsvariable `STENCILCRAFT_HOME`, erstes Elternverzeichnis der EXE mit
/// `Libraries/`-Ordner, sonst das EXE-Verzeichnis.
pub fn resolve_data_root() -> PathBuf {
    if let Ok(home) = std::env::var("STENCILCRAFT_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home);
        }
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    for dir in exe_dir.ancestors().take(8) {
        if dir.join("Libraries").is_dir() {
            return dir.to_path_buf();
        }
    }
    exe_dir
}

/// Kleinste Bohrung, die als THT-Anschluss gilt (kleiner: Via im Pad)
pub const THT_MIN_DRILL: f64 = 0.35;

fn sanitize_file_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect()
}

impl StencilEngine {
    pub fn new(base_thickness: f64, data_root: PathBuf) -> Self {
        let mut e = StencilEngine {
            base_thickness,
            ipc_min_ar: 0.66,
            ipc_min_aspect: 1.50,
            ipc_mode: "standard".into(),
            squeegee_dir: "Y".into(),
            squeegee_clearance: 1.2,
            transverse_clearance: 0.8,
            current_side: "top".into(),
            dataset_name: NO_DATASET.into(),
            original_apertures: Vec::new(),
            apertures: Vec::new(),
            pads: Vec::new(),
            copper_pads: Vec::new(),
            components: Vec::new(),
            packages: BTreeMap::new(),
            package_aliases: BTreeMap::new(),
            pcbi_settings_xml: None,
            current_customer: "Standard".into(),
            zones: Vec::new(),
            profile_paths: Vec::new(),
            fiducials: Vec::new(),
            layers: Vec::new(),
            bounds: [0.0; 4],
            paste_type: "type4".into(),
            min_bridge_width: 0.10,
            bridge_violations: Vec::new(),
            odb_toeprints: Vec::new(),
            drills: Vec::new(),
            source_paths: Vec::new(),
            source_side: "top".into(),
            project_path: None,
            dirty: false,
            history: project::History::default(),
            sides: BTreeMap::new(),
            group_rules: rulesets::default_group_rules(),
            component_rules: BTreeMap::new(),
            data_root,
        };
        e.init_standard_packages();
        e
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.data_root.join("Libraries")
    }

    pub fn export_dir(&self) -> PathBuf {
        self.data_root.join("Export")
    }

    // -------------------------------------------------------------
    // TOP / BOT
    // -------------------------------------------------------------
    /// Nimmt den Arbeitsstand der aktiven Seite heraus (Felder bleiben leer zurück).
    pub fn take_side_data(&mut self) -> SideData {
        SideData {
            apertures: std::mem::take(&mut self.apertures),
            original_apertures: std::mem::take(&mut self.original_apertures),
            zones: std::mem::take(&mut self.zones),
            pads: std::mem::take(&mut self.pads),
            copper_pads: std::mem::take(&mut self.copper_pads),
            odb_toeprints: std::mem::take(&mut self.odb_toeprints),
            bridge_violations: std::mem::take(&mut self.bridge_violations),
            base_thickness: self.base_thickness,
        }
    }

    fn put_side_data(&mut self, d: SideData) {
        self.apertures = d.apertures;
        self.original_apertures = d.original_apertures;
        self.zones = d.zones;
        self.pads = d.pads;
        self.copper_pads = d.copper_pads;
        self.odb_toeprints = d.odb_toeprints;
        self.bridge_violations = d.bridge_violations;
        if d.base_thickness > 0.0 {
            self.base_thickness = d.base_thickness;
        }
    }

    /// Wechselt die aktive Seite ("top" / "bot"). Liefert false, wenn bereits aktiv.
    pub fn switch_side(&mut self, side: &str) -> bool {
        let side = if side.starts_with('b') { "bot" } else { "top" };
        if self.current_side == side {
            return false;
        }
        let current = self.take_side_data();
        let old = std::mem::replace(&mut self.current_side, side.to_string());
        self.sides.insert(old, current);
        let next = self.sides.remove(side).unwrap_or_else(|| SideData { base_thickness: self.base_thickness, ..Default::default() });
        self.put_side_data(next);
        self.apply_side_visibility();
        self.annotate_packages();
        true
    }

    /// Lagen der aktiven Seite einblenden, die der anderen Seite ausblenden.
    pub fn apply_side_visibility(&mut self) {
        let side = self.current_side.clone();
        for l in self.layers.iter_mut() {
            let s = l.side.to_lowercase();
            if s != "top" && s != "bot" {
                continue;
            }
            let own = s == side;
            match l.layer_type.as_str() {
                "paste" | "mask" | "copper" | "component" => l.visible = own && !l.name.to_lowercase().contains("copy"),
                "paste_orig" | "silk" | "other" if !own => l.visible = false,
                _ => {}
            }
        }
    }

    /// Seiten mit Pastendaten (aktive + gespeicherte).
    pub fn sides_json(&self) -> Value {
        let mut list = Vec::new();
        for s in ["top", "bot"] {
            let (n, loaded) = if s == self.current_side {
                (self.apertures.len(), !self.apertures.is_empty())
            } else {
                self.sides.get(s).map_or((0, false), |d| (d.apertures.len(), !d.apertures.is_empty()))
            };
            list.push(json!({"side": s, "apertures": n, "loaded": loaded, "active": s == self.current_side}));
        }
        json!({"active": self.current_side, "sides": list})
    }

    pub fn set_paste_type(&mut self, ptype: &str) -> String {
        let clean = ptype.trim().to_lowercase();
        self.paste_type = if clean.contains('3') {
            "type3"
        } else if clean.contains('5') {
            "type5"
        } else {
            "type4"
        }
        .into();
        self.paste_type.clone()
    }

    pub fn set_min_bridge_width(&mut self, width: f64) -> f64 {
        self.min_bridge_width = width.max(0.04);
        self.min_bridge_width
    }

    pub fn set_ipc_mode(&mut self, mode: &str, custom_ar: f64, custom_aspect: f64) {
        self.ipc_mode = mode.into();
        match mode {
            "standard" => {
                self.ipc_min_ar = 0.66;
                self.ipc_min_aspect = 1.50;
            }
            "nano" => {
                self.ipc_min_ar = 0.55;
                self.ipc_min_aspect = 1.30;
            }
            "custom" => {
                self.ipc_min_ar = custom_ar;
                self.ipc_min_aspect = custom_aspect;
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------
    // Lagen-Manager
    // -------------------------------------------------------------
    pub fn layers_summary(&self) -> Vec<Value> {
        self.layers.iter().map(BoardLayer::summary).collect()
    }

    pub fn layer_mut(&mut self, id: &str) -> Option<&mut BoardLayer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Fügt eine Lage hinzu oder ersetzt eine gleichnamige (wie Python-Dict-Zuweisung).
    pub fn upsert_layer(&mut self, layer: BoardLayer) {
        if let Some(existing) = self.layers.iter_mut().find(|l| l.id == layer.id) {
            *existing = layer;
        } else {
            self.layers.push(layer);
        }
    }

    pub fn update_layer_settings(&mut self, id: &str, visible: Option<bool>, color: Option<String>, opacity: Option<f64>, layer_type: Option<String>) {
        if let Some(l) = self.layer_mut(id) {
            if let Some(v) = visible {
                l.visible = v;
            }
            if let Some(c) = color.filter(|c| !c.is_empty()) {
                l.color = c;
            }
            if let Some(o) = opacity {
                l.opacity = o.clamp(0.0, 1.0);
            }
            if let Some(t) = layer_type.filter(|t| !t.is_empty()) {
                l.layer_type = t;
            }
        }
    }

    pub fn remove_layer(&mut self, id: &str) {
        self.layers.retain(|l| l.id != id);
    }

    pub fn close_dataset(&mut self) {
        self.apertures.clear();
        self.original_apertures.clear();
        self.zones.clear();
        self.pads.clear();
        self.components.clear();
        self.profile_paths.clear();
        self.fiducials.clear();
        self.layers.clear();
        self.packages.clear();
        self.package_aliases.clear();
        self.bridge_violations.clear();
        self.odb_toeprints.clear();
        self.drills.clear();
        self.copper_pads.clear();
        self.source_paths.clear();
        self.project_path = None;
        self.dirty = false;
        self.history.clear();
        self.sides.clear();
        self.component_rules.clear();
        self.bounds = [0.0; 4];
        self.dataset_name = NO_DATASET.into();
        self.init_standard_packages();
    }

    /// Findet zu jeder Pasten-Apertur das korrespondierende Kupfer-Pad (max. 0.5 mm Mittenversatz).
    pub fn correlate_apertures_with_pads(&mut self) {
        if self.pads.is_empty() || self.apertures.is_empty() {
            return;
        }
        let pads = &self.pads;
        for ap in self.apertures.iter_mut() {
            let mut best: Option<&PadBox> = None;
            let mut min_dist = 0.5;
            for p in pads {
                let d = (ap.cx - (p.x0 + p.x1) / 2.0).hypot(ap.cy - (p.y0 + p.y1) / 2.0);
                if d < min_dist {
                    min_dist = d;
                    best = Some(p);
                }
            }
            if let Some(p) = best {
                ap.pad_ref = Some(p.clone());
            }
        }
    }

    /// Ordnet jeder Pasten-Apertur das Kupferpad der Außenlage zu (Zentrum innerhalb des Kupferpads).
    pub fn correlate_apertures_with_copper(&mut self, copper: &[PadBox]) {
        use rstar::{primitives::GeomWithData, RTree};
        let tree = RTree::bulk_load(copper.iter().enumerate().map(|(i, p)| GeomWithData::new([(p.x0 + p.x1) / 2.0, (p.y0 + p.y1) / 2.0], i)).collect());
        for ap in self.apertures.iter_mut() {
            // Größtes Kupferpad, das den Pastenmittelpunkt enthält (nicht die Via-Fläche im Pad)
            let hit = tree
                .nearest_neighbor_iter(&[ap.cx, ap.cy])
                .take(8)
                .map(|n| &copper[n.data])
                .filter(|p| ap.cx >= p.x0 - 1e-6 && ap.cx <= p.x1 + 1e-6 && ap.cy >= p.y0 - 1e-6 && ap.cy <= p.y1 + 1e-6)
                // Kupfer gehört zu genau diesem Pad (nicht eine große Massefläche unter mehreren Pastenpads)
                .filter(|p| p.w >= 0.8 * ap.width && p.h >= 0.8 * ap.height && p.w <= 1.6 * ap.width + 0.3 && p.h <= 1.6 * ap.height + 0.3)
                .max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)));
            ap.copper_ref = hit.cloned();
        }
    }

    pub fn get_effective_thickness(&self, cx: f64, cy: f64) -> (f64, Option<&StencilZone>) {
        for z in &self.zones {
            if z.step && z.contains_point(cx, cy) {
                return (z.thickness, Some(z));
            }
        }
        (self.base_thickness, None)
    }

    fn rebuild_bounds_from_apertures(&mut self) {
        if self.apertures.is_empty() {
            return;
        }
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for a in &self.apertures {
            b[0] = b[0].min(a.bbox[0]);
            b[1] = b[1].min(a.bbox[1]);
            b[2] = b[2].max(a.bbox[2]);
            b[3] = b[3].max(a.bbox[3]);
        }
        self.bounds = [b[0] - 5.0, b[1] - 5.0, b[2] + 5.0, b[3] + 5.0];
    }

    // -------------------------------------------------------------
    // Apertur-Werkzeuge (Board View Schnellaktionen)
    // -------------------------------------------------------------
    fn target_ids(&self, target_id: i64, all_identical: bool) -> Vec<usize> {
        if target_id < 0 || target_id as usize >= self.apertures.len() {
            return Vec::new();
        }
        let t = &self.apertures[target_id as usize];
        if all_identical {
            let grp = t.package_group.clone();
            self.apertures.iter().filter(|a| a.package_group == grp).map(|a| a.id).collect()
        } else {
            vec![t.id]
        }
    }

    pub fn apply_thermal_slicing(&mut self, target_id: i64, grid_x: i64, grid_y: i64, web: f64, edge: f64, all_identical: bool) -> usize {
        let (gx, gy) = (grid_x.max(1) as usize, grid_y.max(1) as usize);
        let mut count = 0;
        for id in self.target_ids(target_id, all_identical) {
            let ap = &mut self.apertures[id];
            let total_w = ap.width - 2.0 * edge;
            let total_h = ap.height - 2.0 * edge;
            if total_w <= 0.0 || total_h <= 0.0 {
                continue;
            }
            let sub_w = (total_w - (gx as f64 - 1.0) * web) / gx as f64;
            let sub_h = (total_h - (gy as f64 - 1.0) * web) / gy as f64;
            if sub_w <= 0.0 || sub_h <= 0.0 {
                continue;
            }
            let sx = ap.cx - total_w / 2.0 + sub_w / 2.0;
            let sy = ap.cy - total_h / 2.0 + sub_h / 2.0;
            let mut slices = Vec::with_capacity(gx * gy);
            for ix in 0..gx {
                for iy in 0..gy {
                    slices.push(Slice::rect(
                        r4(sx + ix as f64 * (sub_w + web)),
                        r4(sy + iy as f64 * (sub_h + web)),
                        r4(sub_w),
                        r4(sub_h),
                    ));
                }
            }
            ap.custom_slices = Some(slices);
            count += 1;
        }
        count
    }

    pub fn apply_reduction(&mut self, target_id: i64, percent: f64, all_identical: bool) -> usize {
        let scale = (1.0 - percent / 100.0).max(0.1).sqrt();
        let ids = self.target_ids(target_id, all_identical);
        for &id in &ids {
            let ap = &mut self.apertures[id];
            let mut s = Slice::rect(ap.cx, ap.cy, r4(ap.width * scale), r4(ap.height * scale));
            if ap.shape_type == "circle" {
                s.shape = Some("circle".into());
            }
            ap.custom_slices = Some(vec![s]);
        }
        ids.len()
    }

    pub fn reset_aperture_modification(&mut self, target_id: i64, all_identical: bool) {
        for id in self.target_ids(target_id, all_identical) {
            let orig = self.original_apertures.get(id).cloned();
            let ap = &mut self.apertures[id];
            // Ergänzte Kupfer-Öffnung: "Original" heißt hier keine Paste
            ap.custom_slices = if ap.from_copper { Some(Vec::new()) } else { None };
            ap.is_manually_overridden = false;
            if let Some(o) = orig {
                ap.width = o.width;
                ap.height = o.height;
                ap.shape_type = o.shape_type;
                ap.points = o.points;
                ap.area = o.area;
                ap.perimeter = o.perimeter;
            }
        }
    }

    pub fn apply_elongated_narrowing(&mut self, target_id: i64, width_red: f64, length_use: f64, corner_r: f64, all_identical: bool) -> usize {
        let ids = self.target_ids(target_id, all_identical);
        let pitches = self.aperture_pitches(&ids);
        let min_bridge = self.min_bridge_width;
        for &id in &ids {
            let pitch = pitches.get(&id).copied();
            let ap = &mut self.apertures[id];
            let (w, h) = (ap.width, ap.height);
            let (nw, nh) = if w <= h {
                (elongated_narrow_width(w, width_red, pitch, min_bridge), h * (length_use / 100.0))
            } else {
                (w * (length_use / 100.0), elongated_narrow_width(h, width_red, pitch, min_bridge))
            };
            let r = corner_r.min(nw / 3.0).min(nh / 3.0).max(0.0);
            let pts = geometry::rounded_rect_pts(ap.cx, ap.cy, nw, nh, r);
            ap.custom_slices = Some(vec![Slice {
                cx: r4(ap.cx),
                cy: r4(ap.cy),
                w: r4(nw),
                h: r4(nh),
                radius: Some(r4(r)),
                pts: Some(round_pts(&pts)),
                shape: Some("polygon".into()),
                kind: Some("elongated".into()),
                ..Default::default()
            }]);
        }
        ids.len()
    }

    pub fn override_single_pad(&mut self, ap_id: i64, data: &Map<String, Value>) -> bool {
        if ap_id < 0 || ap_id as usize >= self.apertures.len() {
            return false;
        }
        let ap = &mut self.apertures[ap_id as usize];
        ap.is_manually_overridden = true;
        if let Some(sl) = data.get("slices") {
            if let Ok(slices) = serde_json::from_value::<Vec<Slice>>(sl.clone()) {
                ap.custom_slices = Some(slices);
            }
        } else if let (Some(w), Some(h)) = (data.get("w").and_then(val_f64), data.get("h").and_then(val_f64)) {
            let mut s = Slice::rect(ap.cx, ap.cy, r4(w), r4(h));
            s.shape = Some("rectangle".into());
            s.manual = Some(true);
            ap.custom_slices = Some(vec![s]);
        }
        true
    }

    /// Pitch (Mittenabstand quer zur Padlänge) jeder Apertur innerhalb ihres Bauteils.
    pub fn aperture_pitches(&self, ids: &[usize]) -> std::collections::HashMap<usize, f64> {
        let mut by_comp: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
        for ap in &self.apertures {
            if !ap.component_id.is_empty() {
                by_comp.entry(ap.component_id.as_str()).or_default().push(ap.id);
            }
        }
        let mut out = std::collections::HashMap::new();
        for &id in ids {
            let ap = &self.apertures[id];
            let Some(group) = by_comp.get(ap.component_id.as_str()) else { continue };
            let pads: Vec<(f64, f64, f64, f64)> = group.iter().map(|&j| {
                let a = &self.apertures[j];
                (a.cx, a.cy, a.width, a.height)
            }).collect();
            let me = group.iter().position(|&j| j == id).unwrap_or(0);
            if let Some(p) = neighbor_pitch(&pads, me) {
                out.insert(id, p);
            }
        }
        out
    }

    /// Regelgeometrie einer Apertur gemäß ihrem Package (inkl. Pin-Override) neu berechnen.
    fn regenerate_from_package(&self, ap: &ApertureItem, pkg: &PackageTemplate, pitch: Option<f64>) -> Option<Vec<Slice>> {
        let (base_rule, _) = self.effective_rule(ap);
        let (rule, pw, ph) = match pkg.pad_overrides.get(&ap.pin_number) {
            Some(ov) => {
                let (mut ow, mut oh) = (ov.get("w").and_then(val_f64), ov.get("h").and_then(val_f64));
                // Override-Maße gelten im Package-Rahmen; liegt die Instanz um 90° gedreht auf der
                // Platine (Template-Pad B×H = Board-Pad H×B), werden sie getauscht.
                if let Some(tp) = pkg.pads_relative.iter().find(|p| p.pin == ap.pin_number) {
                    if is_rotated_instance(tp.w, tp.h, ap.width, ap.height) {
                        std::mem::swap(&mut ow, &mut oh);
                    }
                }
                (resolve_pin_rule(&base_rule, ov), ow.unwrap_or(ap.width), oh.unwrap_or(ap.height))
            }
            None => (base_rule, ap.width, ap.height),
        };
        // Kupfer nur, wenn die Pin-Anpassung keine eigene Größe vorgibt
        let sized = pkg.pad_overrides.get(&ap.pin_number).is_some_and(|o| o.contains_key("w") || o.contains_key("h"));
        let ctx = PadCtx {
            copper: ap.copper_ref.as_ref().filter(|_| !sized).map(|p| (p.w, p.h, p.round)),
            mask: ap.pad_ref.as_ref().map(|p| (p.w, p.h)),
            round: ap.shape_type == "circle",
            pitch,
            min_bridge: self.min_bridge_width,
            pin_count: pkg.pads_relative.len().max(pkg.pin_count),
            thickness: self.get_effective_thickness(ap.cx, ap.cy).0,
            drill: self.drill_at(ap.cx, ap.cy, ap.width.min(ap.height)),
            ..PadCtx::new(pw, ph, ap.cx, ap.cy, &ap.facing_dir)
        };
        generate_rule_aperture_geometry(&ctx, &rule)
    }

    pub fn reset_single_pad(&mut self, ap_id: i64) -> bool {
        if ap_id < 0 || ap_id as usize >= self.apertures.len() {
            return false;
        }
        let id = ap_id as usize;
        self.apertures[id].is_manually_overridden = false;
        let target = self.resolve_alias(&self.apertures[id].package_name);
        let pitch = self.aperture_pitches(&[id]).get(&id).copied();
        let slices = match self.packages.get(&target) {
            Some(pkg) => self.regenerate_from_package(&self.apertures[id], pkg, pitch),
            None => None,
        };
        self.apertures[id].custom_slices = slices;
        true
    }

    pub fn apply_pip_aperture(&mut self, ap_id: i64, pip: &Map<String, Value>) -> bool {
        if ap_id < 0 || ap_id as usize >= self.apertures.len() {
            return false;
        }
        let web = (self.min_bridge_width * 2.0).max(0.15);
        let ap = &mut self.apertures[ap_id as usize];
        let (cx, cy) = (ap.cx, ap.cy);
        let shape = pip.get("shape").and_then(|v| v.as_str()).unwrap_or("rectangle");
        let get = |k: &str, d: f64| pip.get(k).and_then(val_f64).unwrap_or(d);

        match shape {
            "donut" => {
                let d_out = get("d_outer", ap.width * 2.0);
                let d_in = get("d_inner", ap.width * 0.8).min(d_out * 0.95);
                // Ringöffnung in 4 Segmente mit Haltestegen: ein geschlossener Ring würde
                // die innere Schablonen-Insel beim Laserschneiden herausfallen lassen.
                ap.custom_slices = Some(donut_segments(cx, cy, d_in, d_out, web));
            }
            "cross" => {
                let core = get("core_size", 1.2);
                let arm_w = get("arm_width", 0.8);
                let arm_l = get("arm_length", 0.6);
                let total = core + 2.0 * arm_l;
                let (hc, ha, th) = (core / 2.0, arm_w / 2.0, core / 2.0 + arm_l);
                let pts = vec![
                    (cx - ha, cy + th), (cx + ha, cy + th), (cx + ha, cy + hc), (cx + th, cy + hc),
                    (cx + th, cy - hc), (cx + ha, cy - hc), (cx + ha, cy - th), (cx - ha, cy - th),
                    (cx - ha, cy - hc), (cx - th, cy - hc), (cx - th, cy + hc), (cx - ha, cy + hc),
                ];
                ap.custom_slices = Some(vec![Slice {
                    cx,
                    cy,
                    w: r4(total),
                    h: r4(total),
                    pts: Some(round_pts(&pts)),
                    shape: Some("polygon".into()),
                    kind: Some("pip_cross".into()),
                    manual: Some(true),
                    ..Default::default()
                }]);
            }
            _ => {
                let w = get("w", ap.width * 1.5);
                let h = get("h", ap.height * 1.5);
                ap.custom_slices = Some(vec![Slice {
                    cx,
                    cy,
                    w: r4(w),
                    h: r4(h),
                    shape: Some("rectangle".into()),
                    kind: Some("pip_rectangle".into()),
                    manual: Some(true),
                    ..Default::default()
                }]);
            }
        }
        ap.is_manually_overridden = true;
        true
    }

    // -------------------------------------------------------------
    // Stufenzonen
    // -------------------------------------------------------------
    fn next_zone_id(&self) -> usize {
        self.zones
            .iter()
            .filter_map(|z| z.id.strip_prefix("zone_").and_then(|n| n.parse::<usize>().ok()))
            .max()
            .unwrap_or(0)
            + 1
    }

    fn auto_zone_name(n: usize, thickness: f64, is_step_down: bool) -> String {
        format!("Zone {} ({} {}µm)", n, if is_step_down { "Step-Down" } else { "Step-Up" }, (thickness * 1000.0).round() as i64)
    }

    pub fn add_zone(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, thickness: f64, name: &str) -> StencilZone {
        self.add_zone_ext(x0, y0, x1, y1, thickness, name, true)
    }

    /// Zone anlegen; `step = false` → reine Regelzone ohne Dickenänderung.
    #[allow(clippy::too_many_arguments)]
    pub fn add_zone_ext(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, thickness: f64, name: &str, step: bool) -> StencilZone {
        let n = self.next_zone_id();
        let thickness = if step { thickness } else { self.base_thickness };
        let is_step_down = thickness < self.base_thickness;
        let name = if !name.trim().is_empty() {
            name.to_string()
        } else if step {
            Self::auto_zone_name(n, thickness, is_step_down)
        } else {
            format!("Regelzone {n}")
        };
        let z = StencilZone {
            id: format!("zone_{n}"),
            name,
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
            thickness,
            is_step_down,
            step,
            rule: None,
        };
        self.zones.push(z.clone());
        z
    }

    pub fn update_zone(&mut self, id: &str, thickness: f64, name: Option<String>) {
        let base = self.base_thickness;
        if let Some(z) = self.zones.iter_mut().find(|z| z.id == id) {
            let n: usize = z.id.trim_start_matches("zone_").parse().unwrap_or(0);
            // Automatisch vergebene Namen mitführen, damit "Step-Down/Up" und Dicke stimmen
            let was_auto = z.name == Self::auto_zone_name(n, z.thickness, z.is_step_down);
            z.thickness = thickness;
            z.is_step_down = thickness < base;
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                z.name = name;
            } else if was_auto {
                z.name = Self::auto_zone_name(n, thickness, z.is_step_down);
            }
        }
    }

    pub fn delete_zone(&mut self, id: &str) {
        self.zones.retain(|z| z.id != id);
    }

    // -------------------------------------------------------------
    // Package-Regeln (Stencil Generator)
    // -------------------------------------------------------------
    pub fn resolve_alias(&self, name: &str) -> String {
        self.package_aliases.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    pub fn apply_package_rule(&mut self, pkg_name: &str, rule: &Map<String, Value>) -> usize {
        let target = self.resolve_alias(pkg_name);
        self.packages
            .entry(target.clone())
            .or_insert_with(|| PackageTemplate::new(&target, 2, 1.0, 1.0, 1.0))
            .rule
            .apply_map(rule);
        self.regenerate_package_apertures(&target, None)
    }

    /// Berechnet alle (nicht manuell übersteuerten) Aperturen eines Packages neu.
    /// `only_pin` beschränkt die Neuberechnung auf einen Pin (inklusive manueller Pads, wie im Original).
    fn regenerate_package_apertures(&mut self, target: &str, only_pin: Option<i64>) -> usize {
        let Some(pkg) = self.packages.get(target) else { return 0 };
        let aliases = &self.package_aliases;
        let ids: Vec<usize> = self
            .apertures
            .iter()
            .filter(|ap| aliases.get(&ap.package_name).map(String::as_str).unwrap_or(&ap.package_name) == target)
            .filter(|ap| match only_pin {
                Some(pin) => ap.pin_number == pin,
                None => !ap.is_manually_overridden,
            })
            .map(|ap| ap.id)
            .collect();
        let pitches = self.aperture_pitches(&ids);
        let mut updates = Vec::with_capacity(ids.len());
        for &id in &ids {
            updates.push((id, self.regenerate_from_package(&self.apertures[id], pkg, pitches.get(&id).copied())));
        }
        let n = updates.len();
        for (id, slices) in updates {
            self.apertures[id].custom_slices = slices;
        }
        n
    }

    pub fn override_package_pin(&mut self, pkg_name: &str, pin: i64, data: Map<String, Value>) -> bool {
        let target = self.resolve_alias(pkg_name);
        let Some(pkg) = self.packages.get_mut(&target) else { return false };
        pkg.pad_overrides.insert(pin, data);
        self.regenerate_package_apertures(&target, Some(pin));
        true
    }

    /// Übersteuerung für alle Pins eines Packages. Breite/Höhe werden nur auf Pins übertragen, deren
    /// Kupfergröße dem Referenzpin `match_pin` entspricht (gleichartige Pads); ohne Referenz auf alle.
    pub fn override_package_pins_bulk(&mut self, pkg_name: &str, data: &Map<String, Value>, w: Option<f64>, h: Option<f64>, match_pin: Option<i64>) -> usize {
        let target = self.resolve_alias(pkg_name);
        let Some(pkg) = self.packages.get_mut(&target) else { return 0 };
        let reference = match_pin.and_then(|mp| pkg.pads_relative.iter().find(|p| p.pin == mp)).map(|p| (p.w, p.h));
        let pins: Vec<(i64, f64, f64)> = pkg.pads_relative.iter().map(|p| (p.pin, p.w, p.h)).collect();
        let mut clean = data.clone();
        clean.remove("w");
        clean.remove("h");
        for (pin, pw, ph) in &pins {
            let entry = pkg.pad_overrides.entry(*pin).or_default();
            for (k, v) in &clean {
                entry.insert(k.clone(), v.clone());
            }
            let same = reference.map_or(true, |(rw, rh)| (rw - pw).abs() < 0.01 && (rh - ph).abs() < 0.01);
            if same {
                if let Some(w) = w {
                    entry.insert("w".into(), json!(w));
                }
                if let Some(h) = h {
                    entry.insert("h".into(), json!(h));
                }
            }
        }
        self.regenerate_package_apertures(&target, None);
        pins.len()
    }

    pub fn reset_package_pins_all(&mut self, pkg_name: &str) -> bool {
        let target = self.resolve_alias(pkg_name);
        let Some(pkg) = self.packages.get_mut(&target) else { return false };
        pkg.pad_overrides.clear();
        self.regenerate_package_apertures(&target, None);
        true
    }

    pub fn reset_package_pin(&mut self, pkg_name: &str, pin: i64) -> bool {
        let target = self.resolve_alias(pkg_name);
        let Some(pkg) = self.packages.get_mut(&target) else { return false };
        pkg.pad_overrides.remove(&pin);
        self.regenerate_package_apertures(&target, Some(pin));
        true
    }

    pub fn rename_package(&mut self, old: &str, new: &str) -> bool {
        let new = new.trim();
        if new.is_empty() || old == new {
            return false;
        }
        let real_old = self.resolve_alias(old);
        if let Some(mut pkg) = self.packages.remove(&real_old) {
            pkg.name = new.to_string();
            self.packages.insert(new.to_string(), pkg);
        }
        self.package_aliases.insert(old.to_string(), new.to_string());
        self.package_aliases.insert(real_old.clone(), new.to_string());
        // Aliase, die auf den alten Namen zeigten, umlenken (sonst Zyklen/Leichen)
        for v in self.package_aliases.values_mut() {
            if *v == real_old {
                *v = new.to_string();
            }
        }
        self.package_aliases.remove(new);
        for ap in self.apertures.iter_mut() {
            if ap.package_name == old || ap.package_name == real_old {
                ap.package_name = new.to_string();
            }
        }
        for c in self.components.iter_mut() {
            if c.package == old || c.package == real_old {
                c.package = new.to_string();
            }
        }
        true
    }

    pub fn get_package_preview_data(&self, pkg_name: &str) -> Value {
        self.package_preview_with(pkg_name, None)
    }

    /// Vorschau eines Packages; `rule_override` ersetzt die Package-Regel (z. B. Bauteilregel).
    pub fn package_preview_with(&self, pkg_name: &str, rule_override: Option<(&PackageRule, String)>) -> Value {
        let target = self.resolve_alias(pkg_name);
        let fallback;
        let pkg = match self.packages.get(&target) {
            Some(p) => p,
            None => {
                let mut p = PackageTemplate::new(&target, 2, 1.702, 1.20, 1.30);
                p.pads_relative = vec![PadRel::new(1, -0.851, 0.0, 1.20, 1.30, "right"), PadRel::new(2, 0.851, 0.0, 1.20, 1.30, "left")];
                fallback = p;
                &fallback
            }
        };
        let (eff, source) = match &rule_override {
            Some((r, src)) => ((*r).clone(), src.clone()),
            None => self.package_effective_rule(pkg),
        };
        let rel: Vec<(f64, f64, f64, f64)> = pkg.pads_relative.iter().map(|p| (p.cx, p.cy, p.w, p.h)).collect();
        let mut paste = Vec::new();
        for (i, p) in pkg.pads_relative.iter().enumerate() {
            let (rule, pw, ph) = match pkg.pad_overrides.get(&p.pin) {
                Some(ov) => (resolve_pin_rule(&eff, ov), ov.get("w").and_then(val_f64).unwrap_or(p.w), ov.get("h").and_then(val_f64).unwrap_or(p.h)),
                None => (eff.clone(), p.w, p.h),
            };
            let sized = pkg.pad_overrides.get(&p.pin).is_some_and(|o| o.contains_key("w") || o.contains_key("h"));
            let ctx = PadCtx {
                copper: p.copper.filter(|_| !sized),
                round: p.is_round(),
                pitch: neighbor_pitch(&rel, i),
                min_bridge: self.min_bridge_width,
                pin_count: pkg.pads_relative.len(),
                mask: p.mask_w.zip(p.mask_h),
                drill: p.drill,
                thickness: self.base_thickness,
                ..PadCtx::new(pw, ph, p.cx, p.cy, &p.facing)
            };
            if let Some(mut g) = generate_rule_aperture_geometry(&ctx, &rule) {
                for s in g.iter_mut() {
                    s.pin = Some(p.pin);
                }
                paste.extend(g);
            }
        }
        json!({
            "name": pkg.name,
            "pin_count": pkg.pin_count,
            "pitch": pkg.pitch,
            "pad_width": pkg.pad_width,
            "pad_height": pkg.pad_height,
            "category": pkg.category,
            "group": pkg.group,
            "component_count": pkg.component_count,
            "outline": pkg.outline,
            "pads": pkg.pads_relative,
            "paste": paste,
            "rule": pkg.rule.to_json(),
            "effective_rule": eff.to_json(),
            "rule_source": source,
            "base_thickness": self.base_thickness,
            "pad_overrides": pkg.overrides_json(),
        })
    }

    // -------------------------------------------------------------
    // Kunden-Bibliotheken (JSON-kompatibel zur Web-App)
    // -------------------------------------------------------------
    pub fn list_customer_libraries(&self) -> Vec<String> {
        let dir = self.libraries_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut libs: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        let is_json = p.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("json")).unwrap_or(false);
                        if is_json { p.file_stem().and_then(|s| s.to_str()).map(String::from) } else { None }
                    })
                    .collect()
            })
            .unwrap_or_default();
        libs.sort();
        if libs.is_empty() {
            libs.push("Standard".into());
        }
        libs
    }

    pub fn load_customer_library(&mut self, customer: &str) -> bool {
        let file = self.libraries_dir().join(format!("{}.json", sanitize_file_name(customer)));
        let Ok(text) = std::fs::read_to_string(&file) else { return false };
        let Ok(data) = serde_json::from_str::<Value>(&text) else {
            eprintln!("Fehler beim Laden der Kundenbibliothek {customer}: ungültiges JSON");
            return false;
        };
        self.current_customer = data.get("customer").and_then(|v| v.as_str()).unwrap_or(customer).to_string();
        if let Some(pkgs) = data.get("packages").and_then(|v| v.as_object()) {
            for (name, rule) in pkgs {
                if let (Some(pkg), Some(r)) = (self.packages.get_mut(&self.package_aliases.get(name).cloned().unwrap_or_else(|| name.clone())), rule.as_object()) {
                    pkg.rule.apply_map(r);
                }
            }
        }
        if let Some(pins) = data.get("pin_overrides").and_then(|v| v.as_object()) {
            for (name, ov) in pins {
                let real = self.resolve_alias(name);
                let (Some(pkg), Some(ov)) = (self.packages.get_mut(&real), ov.as_object()) else { continue };
                pkg.pad_overrides.clear();
                for (pin, m) in ov {
                    if let (Ok(pin), Some(m)) = (pin.parse::<i64>(), m.as_object()) {
                        pkg.pad_overrides.insert(pin, m.clone());
                    }
                }
            }
        }
        if let Some(s) = data.get("pcbi_settings").and_then(|v| v.as_str()) {
            self.pcbi_settings_xml = Some(s.to_string());
        }
        if let Some(groups) = data.get("groups").and_then(|v| v.as_object()) {
            for (g, rule) in groups {
                if let Some(r) = rule.as_object() {
                    let mut gr = self.group_rule(g);
                    gr.apply_map(r);
                    self.group_rules.insert(g.clone(), gr);
                }
            }
        }
        if let Some(al) = data.get("aliases").and_then(|v| v.as_object()) {
            for (k, v) in al {
                if let Some(s) = v.as_str() {
                    self.package_aliases.insert(k.clone(), s.to_string());
                }
            }
        }
        true
    }

    pub fn save_customer_library(&mut self, customer: &str) -> Result<String, String> {
        let dir = self.libraries_dir();
        std::fs::create_dir_all(&dir).map_err(|e| format!("Bibliotheksordner nicht anlegbar: {e}"))?;
        let name = sanitize_file_name(customer);
        if name.is_empty() {
            return Err("Kundenname darf nicht leer sein".into());
        }
        let file = dir.join(format!("{name}.json"));
        // Vorhandene Einträge behalten: die Bibliothek enthält auch Packages anderer Jobs dieses Kunden
        let old: Value = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let obj = |k: &str| old.get(k).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        let mut packages = obj("packages");
        let mut pins = obj("pin_overrides");
        let mut aliases = obj("aliases");
        for (n, p) in self.packages.iter().filter(|(_, p)| p.category != "Other") {
            packages.insert(n.clone(), p.rule.to_json());
            if p.pad_overrides.is_empty() {
                pins.remove(n);
            } else {
                pins.insert(n.clone(), p.overrides_json());
            }
        }
        for (k, v) in &self.package_aliases {
            aliases.insert(k.clone(), json!(v));
        }
        let mut payload = json!({
            "customer": customer,
            "version": "1.2",
            "packages": packages,
            "pin_overrides": pins,
            "groups": self.group_rules,
            "aliases": aliases,
        });
        if let Some(s) = &self.pcbi_settings_xml {
            payload["pcbi_settings"] = json!(s);
        } else if let Some(s) = old.get("pcbi_settings") {
            payload["pcbi_settings"] = s.clone();
        }
        let text = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        std::fs::write(&file, text).map_err(|e| format!("Fehler beim Schreiben von {}: {e}", file.display()))?;
        self.current_customer = customer.to_string();
        Ok(file.to_string_lossy().to_string())
    }

    // -------------------------------------------------------------
    // Gerber-Export
    // -------------------------------------------------------------
    pub fn export_modified_gerber(&self, path: &Path) -> Result<(), String> {
        let content = gerber::write_stencil(&self.apertures, &format!("Schablone {}", self.current_side.to_uppercase()));
        write_file(path, &content)
    }

    pub fn export_zones_gerber(&self, path: &Path) -> Result<(), String> {
        let content = gerber::write_zones(&self.zones, self.base_thickness);
        write_file(path, &content)
    }
}

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Konnte Zielverzeichnis nicht erstellen: {e}"))?;
    }
    std::fs::write(path, content).map_err(|e| format!("Fehler beim Schreiben von {}: {e}", path.display()))
}

/// Liegt eine Pad-Instanz gegenüber dem Template um 90° gedreht? (Vergleich der Ausrichtung,
/// unabhängig von Undersize/Paste-Reduktion)
pub fn is_rotated_instance(tw: f64, th: f64, aw: f64, ah: f64) -> bool {
    let t_square = (tw - th).abs() <= 0.05 * tw.max(th);
    let a_square = (aw - ah).abs() <= 0.05 * aw.max(ah);
    !t_square && !a_square && (tw > th) != (aw > ah)
}

impl StencilEngine {
    /// Ergänzt Package-Templates um Lötstopp-Freistellung und Bohrung je Pin (aus der ersten Instanz
    /// auf der Platine) und ordnet sie den Gruppen SMD / THT / Other / Fiducial / Library zu.
    pub fn annotate_packages(&mut self) {
        // Erste Board-Instanz je (Package, Pin): Pasten-Apertur (Position, Lage)
        let mut first: std::collections::HashMap<(String, i64), (f64, f64, bool)> = std::collections::HashMap::new();
        let mut instances: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut copper_first: std::collections::HashMap<(String, i64), (f64, f64, bool)> = std::collections::HashMap::new();
        let comp_rot: std::collections::HashMap<&str, f64> = self.components.iter().map(|c| (c.refdes.as_str(), c.rotation)).collect();
        let quarter = |rot: f64| ((rot.rem_euclid(180.0)) - 90.0).abs() < 1.0;
        for ap in &self.apertures {
            let pkg = self.package_aliases.get(&ap.package_name).cloned().unwrap_or_else(|| ap.package_name.clone());
            *instances.entry(pkg.clone()).or_default() += 1;
            let rotated = match comp_rot.get(ap.component_id.as_str()) {
                Some(r) => quarter(*r),
                None => self.packages.get(&pkg).and_then(|t| t.pads_relative.iter().find(|p| p.pin == ap.pin_number)).map_or(false, |tp| is_rotated_instance(tp.w, tp.h, ap.width, ap.height)),
            };
            if let Some(c) = &ap.copper_ref {
                copper_first.entry((pkg.clone(), ap.pin_number)).or_insert(if rotated { (c.h, c.w, c.round) } else { (c.w, c.h, c.round) });
            }
            first.entry((pkg, ap.pin_number)).or_insert((ap.cx, ap.cy, rotated));
        }
        // Bauteile ohne Paste (THT-Stecker): Pinpositionen aus den ODB++ Toeprints
        let comp_pkg: std::collections::HashMap<&str, (&str, f64)> = self.components.iter().map(|c| (c.refdes.as_str(), (c.package.as_str(), c.rotation))).collect();
        for (refdes, pins) in &self.odb_toeprints {
            if let Some(&(pkg, rot)) = comp_pkg.get(refdes.as_str()) {
                for &(pin, x, y) in pins {
                    first.entry((pkg.to_string(), pin)).or_insert((x, y, quarter(rot)));
                }
            }
        }
        let nearest_mask = |x: f64, y: f64| -> Option<&PadBox> {
            self.pads
                .iter()
                .map(|m| (m, ((m.x0 + m.x1) / 2.0 - x).hypot((m.y0 + m.y1) / 2.0 - y)))
                .filter(|(_, d)| *d < 0.5)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(m, _)| m)
        };
        let mut updates: Vec<(String, Vec<(i64, Option<(f64, f64, bool)>, Option<f64>)>)> = Vec::new();
        for (name, pkg) in &self.packages {
            let mut per_pin = Vec::new();
            for p in &pkg.pads_relative {
                let Some(&(x, y, rotated)) = first.get(&(name.clone(), p.pin)) else { continue };
                let mask = nearest_mask(x, y).map(|m| if rotated { (m.h, m.w, m.round) } else { (m.w, m.h, m.round) });
                let tol = (p.w.min(p.h) / 2.0).max(0.3);
                let drill = self
                    .drills
                    .iter()
                    .map(|d| (d, (d.0 - x).hypot(d.1 - y)))
                    .filter(|(_, dist)| *dist <= tol)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(d, _)| d.2);
                per_pin.push((p.pin, mask, drill));
            }
            updates.push((name.clone(), per_pin));
        }
        for (name, per_pin) in updates {
            let has_instances = instances.get(&name).copied().unwrap_or(0) > 0;
            let Some(pkg) = self.packages.get_mut(&name) else { continue };
            for p in pkg.pads_relative.iter_mut() {
                if let Some(&(w, h, round)) = copper_first.get(&(name.clone(), p.pin)) {
                    p.copper = Some((r3(w), r3(h), round));
                }
            }
            let mut drilled = 0usize;
            let pin_total = per_pin.len().max(1);
            for (pin, mask, drill) in per_pin {
                if let Some(p) = pkg.pads_relative.iter_mut().find(|p| p.pin == pin) {
                    if let Some((w, h, round)) = mask {
                        p.mask_w = Some(r3(w));
                        p.mask_h = Some(r3(h));
                        p.mask_round = Some(round);
                    }
                    if let Some(d) = drill {
                        p.drill = Some(r3(d));
                        // Via-in-Pad (kleine Bohrung) ist kein Bauteilanschluss
                        if d >= THT_MIN_DRILL {
                            drilled += 1;
                        }
                    }
                }
            }
            let lname = name.to_lowercase();
            pkg.group = if lname.contains("fiducial") || lname.starts_with("fid") {
                "Fiducial"
            } else if drilled * 2 >= pin_total && drilled > 0 {
                "THT"
            } else if !has_instances && pkg.component_count == 0 {
                "Library"
            } else if !has_instances || (pkg.category == "Other" && pkg.pads_relative.len() <= 1) {
                "Other"
            } else {
                "SMD"
            }
            .into();
        }
    }
}

/// Mittenabstand des nächsten seitlich benachbarten Pads (quer zur Padlänge).
/// `pads` = (cx, cy, w, h); Nachbarn müssen sich entlang der Padlänge überlappen.
pub fn neighbor_pitch(pads: &[(f64, f64, f64, f64)], i: usize) -> Option<f64> {
    let (cx, cy, w, h) = *pads.get(i)?;
    let narrow_is_x = w <= h;
    let long = w.max(h);
    let mut best: Option<f64> = None;
    for (j, &(ox, oy, ow, oh)) in pads.iter().enumerate() {
        if j == i {
            continue;
        }
        let (d_narrow, d_long) = if narrow_is_x { ((ox - cx).abs(), (oy - cy).abs()) } else { ((oy - cy).abs(), (ox - cx).abs()) };
        let other_long = ow.max(oh);
        if d_narrow > 0.01 && d_long < (long + other_long) / 4.0 && best.map_or(true, |b| d_narrow < b) {
            best = Some(d_narrow);
        }
    }
    best
}

/// Segmentierte PiP-Ringöffnung (4 Kreisringsegmente mit radialen Haltestegen der Breite `web`).
pub fn donut_segments(cx: f64, cy: f64, d_in: f64, d_out: f64, web: f64) -> Vec<Slice> {
    let (ri, ro) = (d_in / 2.0, d_out / 2.0);
    if ro - ri < 0.02 {
        let pts = circle_pts(cx, cy, ro, 48);
        return vec![Slice { cx, cy, w: r4(d_out), h: r4(d_out), pts: Some(round_pts(&pts)), shape: Some("polygon".into()), kind: Some("pip_donut".into()), manual: Some(true), ..Default::default() }];
    }
    let seg = 4;
    let quarter = std::f64::consts::TAU / seg as f64;
    let mut out = Vec::with_capacity(seg);
    for k in 0..seg {
        // Steg parallel: halber Winkel des Spalts je Radius
        let ga_o = (web / 2.0 / ro).min(quarter / 3.0);
        let ga_i = (web / 2.0 / ri.max(1e-3)).min(quarter / 3.0);
        let base = std::f64::consts::FRAC_PI_4 + k as f64 * quarter;
        let (o0, o1) = (base + ga_o, base + quarter - ga_o);
        let (i0, i1) = (base + ga_i, base + quarter - ga_i);
        let n = 12;
        let mut pts = Vec::with_capacity(2 * n + 2);
        for j in 0..=n {
            let a = o0 + (o1 - o0) * j as f64 / n as f64;
            pts.push((cx + ro * a.cos(), cy + ro * a.sin()));
        }
        if i1 > i0 {
            for j in (0..=n).rev() {
                let a = i0 + (i1 - i0) * j as f64 / n as f64;
                pts.push((cx + ri * a.cos(), cy + ri * a.sin()));
            }
        }
        let (x0, y0, x1, y1) = geometry::bounds(&pts);
        let (pcx, pcy) = geometry::centroid(&pts);
        out.push(Slice {
            cx: r4(pcx),
            cy: r4(pcy),
            w: r4(x1 - x0),
            h: r4(y1 - y0),
            area: Some(r5(geometry::polygon_area(&pts))),
            pts: Some(round_pts(&pts)),
            shape: Some("polygon".into()),
            kind: Some("pip_donut".into()),
            manual: Some(true),
            d_inner: Some(r4(d_in)),
            d_outer: Some(r4(d_out)),
            ..Default::default()
        });
    }
    out
}
