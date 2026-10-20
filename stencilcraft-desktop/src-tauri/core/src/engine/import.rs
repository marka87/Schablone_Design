//! Import von Gerber-Dateien, Gerber-Archiven/-Ordnern und ODB++ Datensätzen in die Engine.

use super::gerber::{parse_gerber, GerberData};
use super::odb::{self, OdbFiles, PadGeom, MAX_LAYER_PRIMS};
use super::types::*;
use super::StencilEngine;
use rstar::{primitives::GeomWithData, RTree};
use std::path::Path;

const GERBER_EXT: [&str; 12] = [".ger", ".gbr", ".pho", ".art", ".gtp", ".gbp", ".gts", ".gbs", ".gtl", ".gbl", ".gko", ".gm1"];

fn is_gerber_name(name: &str) -> bool {
    let n = name.to_lowercase();
    GERBER_EXT.iter().any(|e| n.ends_with(e))
}

fn base_name(path: &str) -> String {
    path.replace('\\', "/").rsplit('/').next().unwrap_or(path).to_string()
}

fn clean_id(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
}

/// Lagentyp/-farbe aus dem Dateinamen ableiten (Heuristik der Web-App).
fn classify_gerber_name(bn_lower: &str) -> (&'static str, &'static str) {
    let has = |k: &[&str]| k.iter().any(|x| bn_lower.contains(x));
    if has(&["zone", "step"]) {
        ("zone", "#f59e0b")
    } else if has(&["panel", "paste", "stencil", "schablone", "spt", "spb", ".gtp", ".gbp"]) {
        ("paste", "#c084fc")
    } else if has(&["profile", "outline", ".gko", ".gm1"]) {
        ("profile", "#94a3b8")
    } else if has(&["fiducial", "passer"]) {
        ("fiducial", "#f43f5e")
    } else if has(&["smt", "mask", "lötstopp", "smb", ".gts", ".gbs"]) {
        ("mask", "#06b6d4")
    } else if has(&["copper", "top", "bot", "signal", "cu", ".gtl", ".gbl"]) {
        ("copper", "#10b981")
    } else if has(&["comp", "silk", "bestückung"]) {
        ("component", "#facc15")
    } else {
        ("other", "#a855f7")
    }
}

fn side_from_name(bn_lower: &str) -> &'static str {
    if bn_lower.contains("bot") || bn_lower.contains("bottom") || bn_lower.contains("smb") || bn_lower.contains("spb") || bn_lower.ends_with(".gbp") || bn_lower.ends_with(".gbs") || bn_lower.ends_with(".gbl") {
        "bot"
    } else {
        "top"
    }
}

/// Seite einer ODB++ Lage anhand des Namens (inkl. gängiger Kurzformen).
fn odb_side(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("bot") || n.contains("bottom") || matches!(n.as_str(), "smb" | "spb" | "ssb" | "sb" | "b" | "pmb") || n.ends_with("_b") || n.starts_with("b_") {
        "bot"
    } else {
        "top"
    }
}

impl StencilEngine {
    /// Erstellt eine Lage aus Gerber-Inhalt und liefert die geparsten Daten zur Weiterverwendung.
    pub fn add_layer_from_gerber_content(&mut self, file_path: &str, content: &str, layer_type: Option<&str>, color: Option<&str>) -> (BoardLayer, GerberData) {
        let bn = base_name(file_path);
        let bl = bn.to_lowercase();
        let (auto_type, auto_color) = classify_gerber_name(&bl);
        let data = parse_gerber(content);
        let layer = BoardLayer {
            id: format!("layer_{}", clean_id(&bn)),
            name: bn.clone(),
            filename: bn,
            layer_type: layer_type.unwrap_or(auto_type).to_string(),
            side: side_from_name(&bl).into(),
            color: color.unwrap_or(auto_color).to_string(),
            visible: true,
            opacity: 0.85,
            file_path: file_path.to_string(),
            primitives: data.to_prims(MAX_LAYER_PRIMS),
        };
        self.upsert_layer(layer.clone());
        (layer, data)
    }

    pub fn load_paste_data(&mut self, data: &GerberData) {
        self.apertures = data.paste_apertures();
        if let Some(b) = data.bounding_box() {
            self.bounds = b;
        }
        self.original_apertures = self.apertures.clone();
    }

    pub fn load_zones_data(&mut self, data: &GerberData) {
        let mut idx = self.zones.len() + 1;
        for c in data.contours() {
            let (x0, y0, x1, y1) = super::geometry::bounds(&c);
            if x1 - x0 >= 2.5 && y1 - y0 >= 2.5 {
                self.zones.push(StencilZone {
                    id: format!("zone_{idx}"),
                    name: format!("Zone {idx} (Step-Down)"),
                    x0,
                    y0,
                    x1,
                    y1,
                    thickness: 0.100,
                    is_step_down: true,
                    step: true,
                    rule: None,
                });
                idx += 1;
            }
        }
    }

    /// Verarbeitet eine Menge Gerber-Dateien (aus Archiv, Ordner oder Upload) wie die Web-App.
    fn import_gerber_set(&mut self, files: Vec<(String, String)>, side: &str) -> usize {
        let mut imported: Vec<(BoardLayer, GerberData)> = Vec::new();
        for (path, content) in files {
            imported.push(self.add_layer_from_gerber_content(&path, &content, None, None));
        }
        let pick = |t: &[&str], sd: &str| imported.iter().position(|(l, _)| t.contains(&l.layer_type.as_str()) && l.side.contains(sd));
        let side = if side.starts_with('b') { "bot" } else { "top" };
        let other = if side == "top" { "bot" } else { "top" };
        for s in [other, side] {
            let paste = pick(&["paste"], s);
            if s == other && paste.is_none() {
                continue;
            }
            self.apertures.clear();
            self.original_apertures.clear();
            self.zones.clear();
            self.pads.clear();
            self.current_side = s.into();
            if let Some(i) = paste {
                self.load_paste_data(&imported[i].1);
            }
            if let Some(i) = pick(&["zone"], s) {
                self.load_zones_data(&imported[i].1);
            }
            if let Some(i) = pick(&["mask", "copper"], s) {
                self.pads = imported[i].1.pad_boxes();
                self.correlate_apertures_with_pads();
            }
            if paste.is_some() {
                self.detect_packages();
            }
            if s == other {
                let d = self.take_side_data();
                self.sides.insert(s.to_string(), d);
            }
        }
        if let Some(i) = pick(&["profile"], side).or_else(|| pick(&["profile"], "both")) {
            self.profile_paths = imported[i].1.profile_segments();
        }
        if let Some(i) = pick(&["fiducial"], side) {
            self.fiducials = imported[i].1.fiducials();
        }
        self.apply_side_visibility();
        imported.len()
    }

    /// Importiert Dateien, ZIP/TGZ-Archive (ODB++ oder Gerber) oder Ordner. Liefert Anzahl importierter Lagen.
    pub fn import_files_or_zip(&mut self, paths: &[String], side: &str, clear_previous: bool) -> Result<usize, String> {
        if clear_previous {
            self.close_dataset();
        }
        let mut count = 0;
        let mut errors = Vec::new();
        for fp in paths {
            let p = Path::new(fp);
            if !p.exists() {
                errors.push(format!("Nicht gefunden: {fp}"));
                continue;
            }
            let result: Result<usize, String> = if p.is_dir() || odb::is_archive(p) {
                self.import_container(p, side)
            } else {
                std::fs::read(p)
                    .map_err(|e| format!("{fp}: {e}"))
                    .map(|bytes| self.import_single_gerber(fp, &String::from_utf8_lossy(&bytes)))
            };
            match result {
                Ok(n) => count += n,
                Err(e) => errors.push(e),
            }
        }
        if count == 0 && !errors.is_empty() {
            return Err(errors.join("; "));
        }
        self.load_customer_library(&self.current_customer.clone());
        self.annotate_packages();
        if self.tombstone.auto_apply {
            // je Seite anwenden (aktive Seite zuletzt wiederherstellen)
            let active = self.current_side.clone();
            for s in ["top", "bot"] {
                if s != active && self.sides.contains_key(s) {
                    self.switch_side(s);
                    self.auto_apply_tombstone();
                    self.switch_side(&active);
                }
            }
            self.auto_apply_tombstone();
        }
        // Quelle merken (Projektdatei), Verlauf beginnt neu
        if clear_previous {
            self.source_paths = paths.to_vec();
        } else {
            self.source_paths.extend(paths.iter().cloned());
        }
        self.source_side = if side.starts_with('b') { "bot".into() } else { "top".into() };
        self.history.clear();
        Ok(count)
    }

    fn import_single_gerber(&mut self, fp: &str, content: &str) -> usize {
        let (layer, data) = self.add_layer_from_gerber_content(fp, content, None, None);
        match layer.layer_type.as_str() {
            "paste" => {
                self.load_paste_data(&data);
                self.correlate_apertures_with_pads();
                self.detect_packages();
            }
            "zone" => self.load_zones_data(&data),
            "mask" | "copper" => {
                self.pads = data.pad_boxes();
                self.correlate_apertures_with_pads();
            }
            "profile" => self.profile_paths = data.profile_segments(),
            "fiducial" => self.fiducials = data.fiducials(),
            _ => {}
        }
        1
    }

    /// Ordner oder Archiv: ODB++ wenn erkennbar, sonst Sammlung von Gerber-Dateien.
    fn import_container(&mut self, p: &Path, side: &str) -> Result<usize, String> {
        if let Ok(job) = OdbFiles::load(p) {
            if job.files.keys().any(|k| k.ends_with("/features")) {
                return Ok(self.import_odb(&job, side));
            }
        }
        let mut gerbers: Vec<(String, String)> = Vec::new();
        if p.is_dir() {
            let mut stack = vec![p.to_path_buf()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?.flatten() {
                    let ep = e.path();
                    if ep.is_dir() {
                        stack.push(ep);
                    } else if is_gerber_name(&ep.to_string_lossy()) {
                        if let Ok(b) = std::fs::read(&ep) {
                            gerbers.push((ep.to_string_lossy().to_string(), String::from_utf8_lossy(&b).into_owned()));
                        }
                    }
                }
            }
        } else {
            for (name, data) in odb::read_archive_entries(p)? {
                if is_gerber_name(&name) {
                    gerbers.push((format!("{}/{}", p.display(), name), String::from_utf8_lossy(&data).into_owned()));
                }
            }
        }
        if gerbers.is_empty() {
            return Err(format!("Keine ODB++- oder Gerber-Daten in {} gefunden", p.display()));
        }
        gerbers.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(self.import_gerber_set(gerbers, side))
    }

    // -------------------------------------------------------------
    // ODB++
    // -------------------------------------------------------------
    pub fn import_odb(&mut self, job: &OdbFiles, side: &str) -> usize {
        let side = if side.starts_with('b') { "bot" } else { "top" };
        let Some(step) = job.board_step() else { return 0 };
        let step_dir = format!("steps/{step}");
        let eda = job.get(&format!("{step_dir}/eda/data")).map(|c| odb::parse_eda(c)).unwrap_or_default();
        let user_symbols = odb::parse_user_symbols(job);

        // Lagenliste: aus Matrix, sonst aus den vorhandenen Layer-Ordnern
        let mut matrix = job.get("matrix/matrix").map(|m| odb::parse_matrix(m)).unwrap_or_default();
        if matrix.is_empty() {
            let mut names: Vec<String> = job
                .files
                .keys()
                .filter_map(|k| k.strip_prefix(&format!("{step_dir}/layers/")).and_then(|r| r.split('/').next()).map(String::from))
                .collect();
            names.sort();
            names.dedup();
            matrix = names
                .into_iter()
                .enumerate()
                .map(|(i, n)| {
                    let t = if n.contains("paste") || n == "spt" || n == "spb" {
                        "SOLDER_PASTE"
                    } else if n.contains("mask") || n == "smt" || n == "smb" {
                        "SOLDER_MASK"
                    } else if n.contains("comp") {
                        "COMPONENT"
                    } else if n.contains("silk") {
                        "SILK_SCREEN"
                    } else {
                        "SIGNAL"
                    };
                    odb::MatrixLayer { raw_name: n.clone(), name: n, ltype: t.into(), row: i as i64, start: String::new(), end: String::new(), add_type: String::new() }
                })
                .collect();
        }
        let signal_rows: Vec<&str> = matrix.iter().filter(|l| l.ltype == "SIGNAL" || l.ltype == "POWER_GROUND" || l.ltype == "MIXED").map(|l| l.name.as_str()).collect();
        let (outer_top, outer_bot) = (signal_rows.first().copied(), signal_rows.last().copied());

        let mut overall = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut grow = |b: [f64; 4]| {
            overall = [overall[0].min(b[0]), overall[1].min(b[1]), overall[2].max(b[2]), overall[3].max(b[3])];
        };
        let mut count = 0;
        // Je Seite: Pastenlage, Lötstopp-Pads und Toeprints (Pinpositionen)
        let mut paste_layers: std::collections::HashMap<&'static str, String> = std::collections::HashMap::new();
        let mut side_masks: std::collections::HashMap<&'static str, Vec<PadBox>> = std::collections::HashMap::new();
        let mut side_copper: std::collections::HashMap<&'static str, Vec<PadBox>> = std::collections::HashMap::new();
        let mut side_pours: std::collections::HashMap<&'static str, Vec<Vec<Pt>>> = std::collections::HashMap::new();
        let mut side_traces: std::collections::HashMap<&'static str, Vec<[f64; 5]>> = std::collections::HashMap::new();
        let mut side_toeprints: std::collections::HashMap<&'static str, Vec<(String, Vec<(i64, f64, f64)>)>> = std::collections::HashMap::new();

        for ml in &matrix {
            let name = ml.name.as_str();
            let lt = ml.ltype.as_str();
            let lside = odb_side(name);
            // Rigid-Flex: Coverlay ist nicht die Lötstoppmaske der Pads → als Zusatzlage führen
            let coverlay = ml.add_type == "COVERLAY";
            let (ltype, color, lside) = if coverlay || (lt == "MASK" && !matches!(name, "smt" | "smb" | "top_solder" | "bottom_solder")) {
                // Generischer Typ MASK: Bereichs-/Versteifungslagen (BEND_AREA, FLEX_AREA, STIFFENER, …)
                ("other", "#a855f7", lside)
            } else if lt.contains("PASTE") || matches!(name, "spt" | "spb" | "top_paste" | "bottom_paste") {
                if !name.contains("copy") {
                    paste_layers.entry(lside).or_insert_with(|| name.to_string());
                }
                continue;
            } else if lt.contains("MASK") || matches!(name, "smt" | "smb" | "top_solder" | "bottom_solder") {
                ("mask", "#06b6d4", lside)
            } else if lt == "SIGNAL" || lt == "POWER_GROUND" || lt == "MIXED" {
                let s = if Some(name) == outer_top { "top" } else if Some(name) == outer_bot { "bot" } else { "inner" };
                ("copper", "#10b981", s)
            } else if lt.contains("COMPONENT") || name.contains("comp") {
                ("component", "#facc15", lside)
            } else if lt.contains("ROUT") || matches!(name, "outline" | "profile") {
                ("profile", "#94a3b8", "both")
            } else if lt.contains("DRILL") {
                ("drill", "#ef4444", "both")
            } else if lt.contains("SILK") {
                ("silk", "#f1f5f9", lside)
            } else {
                ("other", "#a855f7", lside)
            };

            let base = format!("{step_dir}/layers/{name}");
            let mut prims: Vec<Prim> = Vec::new();
            if ltype == "component" {
                if let Some(c) = job.get(&format!("{base}/components")) {
                    let comps = odb::parse_components(c);
                    let comp_side = if lside == "bot" { "BOT" } else { "TOP" };
                    for c in &comps {
                        let (pkg, w, h) = usize::try_from(c.pkg_ref).ok().and_then(|i| eda.get(i)).map(|p| (p.name.clone(), p.w, p.h)).unwrap_or(("GENERIC".into(), 1.6, 1.0));
                        prims.push(Prim::Comp { refdes: c.refdes.clone(), pkg: pkg.clone(), x: r3(c.x), y: r3(c.y), w, h, rot: c.rot });
                        self.components.push(ComponentItem {
                            refdes: c.refdes.clone(),
                            package: pkg,
                            cx: r3(c.x),
                            cy: r3(c.y),
                            width: w,
                            height: h,
                            rotation: c.rot,
                            side: comp_side.into(),
                            outline: Vec::new(),
                        });
                    }
                    side_toeprints.entry(if comp_side == "BOT" { "bot" } else { "top" }).or_default().extend(comps.into_iter().map(|c| (c.refdes, c.pins)));
                }
            }
            if let Some(feat) = job.get(&format!("{base}/features")) {
                let f = odb::parse_features_ext(feat, MAX_LAYER_PRIMS, &user_symbols);
                // Durchgangsbohrungen und Lötstopp der aktiven Seite für Pin-Infos merken.
                // Blind-/Microvias (z. B. Lage 1→2 bei HDI/Rigid-Flex) machen ein Pad nicht zum THT-Pin.
                let through = (ml.start.is_empty() && ml.end.is_empty())
                    || (Some(ml.start.as_str()) == outer_top && Some(ml.end.as_str()) == outer_bot)
                    || (Some(ml.start.as_str()) == outer_bot && Some(ml.end.as_str()) == outer_top);
                if ltype == "drill" && through {
                    for p in &f.pads {
                        if let PadGeom::Circle { d } = p.geom {
                            self.drills.push((p.x, p.y, d));
                        }
                    }
                } else if (ltype == "mask" || ltype == "copper") && !coverlay && (lside == "top" || lside == "bot") && !name.contains("copy") {
                    if ltype == "copper" {
                        let key = if lside == "bot" { "bot" } else { "top" };
                        side_pours.entry(key).or_default().extend(f.surfaces.iter().cloned());
                        let tr = side_traces.entry(key).or_default();
                        for p in &f.prims {
                            if let Prim::Line { x1, y1, x2, y2, w } = p {
                                tr.push([*x1, *y1, *x2, *y2, *w]);
                            }
                        }
                    }
                    let target = if ltype == "mask" { &mut side_masks } else { &mut side_copper };
                    for p in &f.pads {
                        let (b, round) = match &p.geom {
                            PadGeom::Circle { d } => ((p.x - d / 2.0, p.y - d / 2.0, p.x + d / 2.0, p.y + d / 2.0), true),
                            PadGeom::Rect { w, h } => ((p.x - w / 2.0, p.y - h / 2.0, p.x + w / 2.0, p.y + h / 2.0), false),
                            PadGeom::Poly { pts } => (super::geometry::bounds(pts), false),
                        };
                        let mut pb = PadBox::new(b.0, b.1, b.2, b.3);
                        pb.round = round;
                        target.entry(if lside == "bot" { "bot" } else { "top" }).or_default().push(pb);
                    }
                }
                if let Some(b) = f.bbox {
                    if b[0] < b[2] {
                        grow(b);
                    }
                }
                if prims.is_empty() {
                    prims = f.prims;
                } else {
                    prims.extend(f.prims.into_iter().take(5000));
                }
            }
            let not_copy = !name.contains("copy");
            let visible = not_copy
                && !coverlay
                && match ltype {
                    "component" | "mask" => lside == side,
                    "copper" => lside == side,
                    "profile" => true,
                    _ => false,
                };
            self.upsert_layer(BoardLayer {
                id: format!("odb_{}", clean_id(name)),
                name: ml.raw_name.clone(),
                filename: name.to_string(),
                layer_type: ltype.into(),
                side: lside.into(),
                color: color.into(),
                visible,
                opacity: 0.85,
                file_path: String::new(),
                primitives: prims,
            });
            count += 1;
        }

        // Platinenkontur aus steps/<step>/profile
        if let Some(prof) = job.get(&format!("{step_dir}/profile")) {
            let f = odb::parse_features(prof, MAX_LAYER_PRIMS);
            let mut segs = Vec::new();
            let mut prims = Vec::new();
            for s in &f.surfaces {
                for i in 0..s.len() {
                    let (a, b) = (s[i], s[(i + 1) % s.len()]);
                    segs.push([(r2(a.0), r2(a.1)), (r2(b.0), r2(b.1))]);
                    prims.push(Prim::line(a.0, a.1, b.0, b.1, 0.15));
                }
            }
            if !prims.is_empty() {
                self.profile_paths = segs;
                if let Some(b) = f.bbox {
                    grow(b);
                }
                self.upsert_layer(BoardLayer {
                    id: "odb_profile".into(),
                    name: "PROFILE (Platinenkontur)".into(),
                    filename: "profile".into(),
                    layer_type: "profile".into(),
                    side: "both".into(),
                    color: "#94a3b8".into(),
                    visible: true,
                    opacity: 0.85,
                    file_path: String::new(),
                    primitives: prims,
                });
                count += 1;
            }
        }

        // Pastenlagen beider Seiten als Arbeits-Aperturen laden; die gewünschte Seite zuletzt (= aktiv)
        let other = if side == "top" { "bot" } else { "top" };
        let layers_prefix = format!("{step_dir}/layers/");
        for s in [other, side] {
            let pl = paste_layers.get(s).cloned().or_else(|| {
                let mut cands: Vec<String> = job
                    .files
                    .keys()
                    .filter_map(|k| k.strip_prefix(&layers_prefix).and_then(|r| r.strip_suffix("/features")).map(String::from))
                    .filter(|n| (n.contains("paste") || n == "spt" || n == "spb") && odb_side(n) == s)
                    .collect();
                cands.sort();
                cands.into_iter().next()
            });
            // Seite ohne Kundenpaste, aber mit Bauteilen: trotzdem anlegen (Öffnungen aus dem Kupfer möglich)
            if s == other && pl.is_none() && side_toeprints.get(s).map_or(true, |t| t.is_empty()) {
                continue;
            }
            self.apertures.clear();
            self.original_apertures.clear();
            self.zones.clear();
            self.bridge_violations.clear();
            self.current_side = s.into();
            self.pads = side_masks.remove(s).unwrap_or_default();
            self.odb_toeprints = side_toeprints.remove(s).unwrap_or_default();
            if let Some(pl) = &pl {
                if let Some(feat) = job.get(&format!("{step_dir}/layers/{pl}/features")) {
                    self.load_odb_paste(&odb::parse_features_ext(feat, usize::MAX, &user_symbols), &eda);
                }
            }
            self.correlate_apertures_with_pads();
            self.copper_pads = side_copper.get(s).cloned().unwrap_or_default();
            self.set_copper_geometry(side_pours.remove(s).unwrap_or_default(), side_traces.remove(s).unwrap_or_default());
            if !self.copper_pads.is_empty() {
                let cu = std::mem::take(&mut self.copper_pads);
                self.correlate_apertures_with_copper(&cu);
                self.copper_pads = cu;
            }

            let orig_prims: Vec<Prim> = self
                .original_apertures
                .iter()
                .map(|a| match a.shape_type.as_str() {
                    "circle" => Prim::circle(a.cx, a.cy, a.width / 2.0),
                    "polygon" => Prim::poly(&a.points),
                    _ => Prim::rect(a.cx, a.cy, a.width, a.height),
                })
                .collect();
            self.upsert_layer(BoardLayer {
                id: format!("paste_orig_{s}"),
                name: format!("{}_PASTE (Kunde / Original)", s.to_uppercase()),
                filename: format!("{s}_paste_orig"),
                layer_type: "paste_orig".into(),
                side: s.into(),
                color: "#ec4899".into(),
                visible: false,
                opacity: 0.80,
                file_path: String::new(),
                primitives: orig_prims,
            });
            self.upsert_layer(BoardLayer {
                id: format!("stencil_mod_{s}"),
                name: format!("Schablone {} (Zwischenspeicher)", s.to_uppercase()),
                filename: format!("stencil_{s}_working_buffer"),
                layer_type: "paste".into(),
                side: s.into(),
                color: "#c084fc".into(),
                visible: s == side,
                opacity: 0.90,
                file_path: String::new(),
                primitives: Vec::new(),
            });
            count += 2;
            if s == other {
                let d = self.take_side_data();
                self.sides.insert(s.to_string(), d);
            }
        }
        self.apply_side_visibility();

        if overall[0] < overall[2] {
            self.bounds = overall;
        }
        self.dataset_name = format!("{} ({})", job.job_name, step);
        count
    }

    /// Erzeugt Arbeits-Aperturen aus einer ODB++ Pastenlage und ordnet sie Bauteilen/Pins zu.
    fn load_odb_paste(&mut self, f: &odb::Features, eda: &[odb::EdaPackage]) {
        self.apertures.clear();
        for pad in &f.pads {
            let id = self.apertures.len();
            let ap = match &pad.geom {
                PadGeom::Circle { d } => Some(ApertureItem::new(id, "circle", pad.x, pad.y, *d, *d)),
                PadGeom::Rect { w, h } => {
                    let mut a = ApertureItem::new(id, "rectangle", pad.x, pad.y, *w, *h);
                    a.points = super::geometry::rect_pts(pad.x, pad.y, *w, *h);
                    Some(a)
                }
                PadGeom::Poly { pts } => ApertureItem::from_polygon(id, pts),
            };
            if let Some(a) = ap {
                self.apertures.push(a);
            }
        }
        for s in &f.surfaces {
            let id = self.apertures.len();
            if let Some(a) = ApertureItem::from_polygon(id, s) {
                if a.area > 1e-5 {
                    self.apertures.push(a);
                }
            }
        }

        self.assign_odb_components(eda);
        self.rebuild_bounds_from_apertures();
        self.original_apertures = self.apertures.clone();
    }

    fn assign_odb_components(&mut self, eda: &[odb::EdaPackage]) {
        // Package-Templates aus EDA-Daten registrieren
        let mut comp_count: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for c in &self.components {
            *comp_count.entry(c.package.as_str()).or_default() += 1;
        }
        for p in eda {
            let pins: Vec<PadRel> = p
                .pins
                .iter()
                .map(|pin| {
                    let facing = if pin.x.abs() < 0.05 && pin.y.abs() < 0.05 {
                        "center"
                    } else if pin.x.abs() >= pin.y.abs() {
                        if pin.x < 0.0 { "right" } else { "left" }
                    } else if pin.y < 0.0 {
                        "up"
                    } else {
                        "down"
                    };
                    let mut pr = PadRel::new(pin.pin, pin.x, pin.y, pin.w, pin.h, facing).round(pin.round);
                    pr.is_thermal = facing == "center" && pin.w >= 1.0 && pin.h >= 1.0;
                    pr
                })
                .collect();
            let pitch = if p.pitch > 0.0 { p.pitch } else { 1.0 };
            let (pw, ph) = pins.first().map(|x| (x.w, x.h)).unwrap_or((1.0, 1.0));
            let mut t = PackageTemplate::new(&p.name, pins.len().max(1), pitch, pw, ph);
            t.category = super::packages::categorize_package_name(&p.name).into();
            t.outline = Outline { w: p.w, h: p.h };
            t.component_count = comp_count.get(p.name.as_str()).copied().unwrap_or(0);
            // Wie PCB Investigator: Standardregel "Inherit – Handle Stencil Creation by Package Group"
            t.rule = PackageRule::with("inherit", "Inherit");
            t.pads_relative = if pins.is_empty() {
                vec![PadRel::new(1, r3(-pitch / 2.0), 0.0, pw, ph, "right"), PadRel::new(2, r3(pitch / 2.0), 0.0, pw, ph, "left")]
            } else {
                pins
            };
            self.packages.insert(p.name.clone(), t);
        }

        if self.components.is_empty() {
            // Keine Bauteildaten: geometrische Erkennung wie beim Gerber-Import
            self.detect_packages();
            return;
        }

        // Toeprint-Index (exakte Pin-Positionen) und Bauteilzentren
        let side = self.current_side.clone();
        let comps: Vec<ComponentItem> = self.components.iter().filter(|c| c.side.eq_ignore_ascii_case(&side)).cloned().collect();
        let comp_idx: std::collections::HashMap<&str, usize> = comps.iter().enumerate().map(|(i, c)| (c.refdes.as_str(), i)).collect();
        let mut tp_points = Vec::new();
        for (refdes, pins) in &self.odb_toeprints {
            if let Some(&ci) = comp_idx.get(refdes.as_str()) {
                for &(pin, x, y) in pins {
                    tp_points.push(GeomWithData::new([x, y], (ci, pin)));
                }
            }
        }
        let tp_tree = RTree::bulk_load(tp_points);
        for ap in self.apertures.iter_mut() {
            let tol = (ap.width.max(ap.height) / 2.0).max(0.3);
            let hit = tp_tree.nearest_neighbor(&[ap.cx, ap.cy]).filter(|n| {
                let p = n.geom();
                (p[0] - ap.cx).hypot(p[1] - ap.cy) <= tol
            });
            let (ci, pin) = match hit {
                Some(n) => (Some(n.data.0), n.data.1),
                None => {
                    let mut best = None;
                    let mut min_d = 4.0;
                    for (i, c) in comps.iter().enumerate() {
                        let d = (c.cx - ap.cx).hypot(c.cy - ap.cy);
                        if d < min_d {
                            min_d = d;
                            best = Some(i);
                        }
                    }
                    (best, 1)
                }
            };
            match ci {
                Some(i) => {
                    let c = &comps[i];
                    ap.package_name = c.package.clone();
                    ap.component_id = c.refdes.clone();
                    ap.pin_number = pin;
                    let (dx, dy) = (c.cx - ap.cx, c.cy - ap.cy);
                    ap.facing_dir = if dx.abs() < 0.05 && dy.abs() < 0.05 {
                        "center"
                    } else if dx.abs() >= dy.abs() {
                        if dx > 0.0 { "right" } else { "left" }
                    } else if dy > 0.0 {
                        "up"
                    } else {
                        "down"
                    }
                    .into();
                }
                None => {
                    ap.package_name = format!("{:.2}x{:.2}", ap.width, ap.height);
                    ap.facing_dir = "center".into();
                }
            }
        }
        // Nicht zugeordnete Einzelpads als Generic-Packages verfügbar machen
        for ap in &self.apertures {
            if !self.packages.contains_key(&ap.package_name) {
                let mut t = PackageTemplate::new(&ap.package_name, 1, 0.0, ap.width, ap.height);
                t.category = "Other".into();
                t.component_count = 0;
                t.pads_relative = vec![PadRel::new(1, 0.0, 0.0, ap.width, ap.height, "center").round(ap.shape_type == "circle")];
                self.packages.insert(ap.package_name.clone(), t);
            }
        }
    }

    /// Demo-/Paneldatensatz aus einzelnen Gerber-Dateien (Pendant zu `load_panel_dataset`).
    pub fn load_panel_dataset(&mut self, paste: &Path, zone: Option<&Path>, pad: Option<&Path>, profile: Option<&Path>, fiducial: Option<&Path>) -> Result<(), String> {
        let read = |p: &Path| std::fs::read(p).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("{}: {e}", p.display()));
        self.close_dataset();
        let fname = paste.to_string_lossy().to_lowercase();
        if let Some(t) = fname.rsplit('_').next().and_then(|s| s.strip_suffix(".ger")).and_then(|s| s.parse::<u32>().ok()) {
            if (10..=999).contains(&t) {
                self.base_thickness = t as f64 / 1000.0;
            }
        }
        if let Some(z) = zone.filter(|p| p.exists()) {
            let (_, d) = self.add_layer_from_gerber_content(&z.to_string_lossy(), &read(z)?, Some("zone"), Some("#f59e0b"));
            self.load_zones_data(&d);
        }
        let (_, d) = self.add_layer_from_gerber_content(&paste.to_string_lossy(), &read(paste)?, Some("paste"), Some("#c084fc"));
        self.load_paste_data(&d);
        if let Some(p) = pad.filter(|p| p.exists()) {
            let is_mask = p.to_string_lossy().to_lowercase().contains("sm");
            let (_, d) = self.add_layer_from_gerber_content(&p.to_string_lossy(), &read(p)?, Some(if is_mask { "mask" } else { "copper" }), Some(if is_mask { "#06b6d4" } else { "#10b981" }));
            self.pads = d.pad_boxes();
        }
        if let Some(p) = profile.filter(|p| p.exists()) {
            let (_, d) = self.add_layer_from_gerber_content(&p.to_string_lossy(), &read(p)?, Some("profile"), Some("#94a3b8"));
            self.profile_paths = d.profile_segments();
        }
        if let Some(p) = fiducial.filter(|p| p.exists()) {
            let (_, d) = self.add_layer_from_gerber_content(&p.to_string_lossy(), &read(p)?, Some("fiducial"), Some("#f43f5e"));
            self.fiducials = d.fiducials();
        }
        self.correlate_apertures_with_pads();
        self.detect_packages();
        self.load_customer_library(&self.current_customer.clone());
        Ok(())
    }
}
