//! Package-Bibliothek: Standard-Footprints, geometrische Package-Erkennung und Auto-Klassifizierung.

use super::types::*;
use super::StencilEngine;
use rstar::{RTree, RTreeObject, AABB};
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};

struct ApEnv {
    id: usize,
    env: AABB<[f64; 2]>,
}

impl RTreeObject for ApEnv {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.env
    }
}

pub fn categorize_package_name(name: &str) -> &'static str {
    let nl = name.to_lowercase();
    let any = |keys: &[&str]| keys.iter().any(|k| nl.contains(k));
    if any(&["0201", "0402", "0603", "0805", "1206", "1210", "cap", "res", "dio", "melf", "ko", "wi"]) {
        "SMD Passives"
    } else if any(&["qfn", "qfp", "soic", "sop", "sot", "tssop", "dfn", "ic", "to-2"]) {
        "ICs & Transistors"
    } else if any(&["bga", "lga", "csp", "array"]) {
        "BGA & Arrays"
    } else if any(&["harting", "phoenix", "te_", "conn", "molex", "relais", "cvilux"]) {
        "Connectors"
    } else {
        "Other"
    }
}

fn two_pin(name: &str, pitch: f64, w: f64, h: f64, ow: f64, oh: f64, undersize: f64) -> PackageTemplate {
    let mut p = PackageTemplate::new(name, 2, pitch, w, h);
    p.category = "SMD Passives".into();
    p.outline = Outline { w: ow, h: oh };
    p.pads_relative = vec![PadRel::new(1, -pitch / 2.0, 0.0, w, h, "right"), PadRel::new(2, pitch / 2.0, 0.0, w, h, "left")];
    p.rule = PackageRule { undersize_w: undersize, undersize_h: undersize, ..Default::default() };
    p
}

fn dual_row(name: &str, n_side: usize, pitch: f64, row_y: f64, w: f64, h: f64, ow: f64, oh: f64) -> PackageTemplate {
    let mut p = PackageTemplate::new(name, n_side * 2, pitch, w, h);
    p.category = "ICs & Transistors".into();
    p.outline = Outline { w: ow, h: oh };
    let x0 = -(n_side as f64 - 1.0) * pitch / 2.0;
    for i in 0..n_side {
        p.pads_relative.push(PadRel::new(i as i64 + 1, r3(x0 + i as f64 * pitch), -row_y, w, h, "up"));
    }
    for i in 0..n_side {
        p.pads_relative.push(PadRel::new((i + n_side) as i64 + 1, r3(-x0 - i as f64 * pitch), row_y, w, h, "down"));
    }
    p.rule = PackageRule { name: "Länglich & Schmal (Pitch-Schutz)".into(), rule_type: "elongated_pitch".into(), ..Default::default() };
    p
}

impl StencilEngine {
    pub fn init_standard_packages(&mut self) {
        self.packages.clear();
        for p in [
            two_pin("0201", 0.50, 0.30, 0.30, 0.60, 0.30, 0.015),
            two_pin("0402", 0.90, 0.50, 0.50, 1.00, 0.50, 0.02),
            two_pin("0603", 1.30, 0.80, 0.80, 1.60, 0.80, 0.025),
            two_pin("0805", 1.702, 1.20, 1.30, 2.00, 1.25, 0.025),
            two_pin("1206", 2.80, 1.60, 1.50, 3.20, 1.60, 0.03),
        ] {
            self.packages.insert(p.name.clone(), p);
        }

        let mut sot = PackageTemplate::new("SOT-23", 3, 0.95, 0.60, 0.80);
        sot.category = "ICs & Transistors".into();
        sot.outline = Outline { w: 2.90, h: 1.30 };
        sot.pads_relative = vec![
            PadRel::new(1, -0.95, -1.0, 0.60, 0.80, "up"),
            PadRel::new(2, 0.95, -1.0, 0.60, 0.80, "up"),
            PadRel::new(3, 0.0, 1.0, 0.60, 0.80, "down"),
        ];
        sot.rule = PackageRule { undersize_w: 0.02, undersize_h: 0.02, ..Default::default() };
        self.packages.insert(sot.name.clone(), sot);

        for p in [
            dual_row("SOIC-8", 4, 1.27, 2.2, 0.60, 1.50, 4.90, 3.90),
            dual_row("SOIC-16", 8, 1.27, 2.2, 0.60, 1.50, 9.90, 3.90),
            dual_row("TSSOP-16", 8, 0.65, 2.7, 0.35, 1.20, 5.00, 4.40),
        ] {
            self.packages.insert(p.name.clone(), p);
        }

        let mut qfn = PackageTemplate::new("QFN-48", 49, 0.50, 0.28, 0.80);
        qfn.category = "ICs & Transistors".into();
        qfn.outline = Outline { w: 7.0, h: 7.0 };
        let mut pin = 1;
        for i in 0..12 {
            qfn.pads_relative.push(PadRel::new(pin, -3.4, 2.75 - i as f64 * 0.5, 0.80, 0.28, "right"));
            pin += 1;
        }
        for i in 0..12 {
            qfn.pads_relative.push(PadRel::new(pin, -2.75 + i as f64 * 0.5, -3.4, 0.28, 0.80, "up"));
            pin += 1;
        }
        for i in 0..12 {
            qfn.pads_relative.push(PadRel::new(pin, 3.4, -2.75 + i as f64 * 0.5, 0.80, 0.28, "left"));
            pin += 1;
        }
        for i in 0..12 {
            qfn.pads_relative.push(PadRel::new(pin, 2.75 - i as f64 * 0.5, 3.4, 0.28, 0.80, "down"));
            pin += 1;
        }
        let mut thermal = PadRel::new(49, 0.0, 0.0, 4.50, 4.50, "center");
        thermal.is_thermal = true;
        qfn.pads_relative.push(thermal);
        qfn.rule = PackageRule { name: "HouseBuilder".into(), rule_type: "house_builder".into(), ..Default::default() };
        self.packages.insert(qfn.name.clone(), qfn);

        let mut bga_pads = Vec::new();
        let mut bp = 1;
        for row in 0..15 {
            for col in 0..15 {
                if (row < 2 || row > 12) && (col < 2 || col > 12) {
                    continue;
                }
                bga_pads.push(PadRel::new(bp, r3((col as f64 - 7.0) * 0.65), r3((row as f64 - 7.0) * 0.65), 0.25, 0.25, "center"));
                bp += 1;
            }
        }
        let mut bga = PackageTemplate::new("BGA-201", bga_pads.len(), 0.65, 0.25, 0.25);
        bga.category = "BGA & Arrays".into();
        bga.outline = Outline { w: 10.0, h: 10.0 };
        bga.pads_relative = bga_pads.into_iter().map(|p| p.round(true)).collect();
        bga.rule = PackageRule { undersize_w: 0.015, undersize_h: 0.015, ..Default::default() };
        self.packages.insert(bga.name.clone(), bga);

        self.package_aliases = [
            ("R0201", "0201"), ("C0201", "0201"),
            ("R0402", "0402"), ("C0402", "0402"), ("C0402X", "0402"),
            ("R0603", "0603"), ("C0603", "0603"), ("BLM18A", "0603"), ("BLM18P", "0603"), ("BLM18PG", "0603"),
            ("R0805", "0805"), ("C0805", "0805"), ("DLW210-0805", "0805"),
            ("R1206", "1206"), ("C1206", "1206"),
            ("BGA201", "BGA-201"), ("BGA201C65P15X15_1000X1000X60", "BGA-201"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        for p in self.packages.values_mut() {
            p.component_count = 0;
        }
    }

    /// Ordnet Aperturen geometrisch Footprints zu (BGA-Feld, 2-Pin-Chips, Cluster für ICs/Steckverbinder).
    pub fn detect_packages(&mut self) {
        if self.apertures.is_empty() {
            return;
        }
        for pkg in self.packages.values_mut() {
            pkg.component_count = 0;
        }
        let tree = RTree::bulk_load(
            self.apertures
                .iter()
                .map(|a| ApEnv { id: a.id, env: AABB::from_corners([a.bbox[0], a.bbox[1]], [a.bbox[2], a.bbox[3]]) })
                .collect(),
        );
        let query = |cx: f64, cy: f64, r: f64| -> Vec<usize> {
            tree.locate_in_envelope_intersecting(&AABB::from_corners([cx - r, cy - r], [cx + r, cy + r])).map(|e| e.id).collect()
        };

        let mut used: HashSet<usize> = HashSet::new();
        let mut comp_counter = 1usize;

        // 1. BGA-Feld (viele kleine Pads)
        let small: Vec<usize> = self.apertures.iter().filter(|a| a.width.max(a.height) <= 0.35).map(|a| a.id).collect();
        if small.len() >= 150 {
            for &id in &small {
                let ap = &mut self.apertures[id];
                ap.package_name = "BGA-201".into();
                ap.component_id = comp_counter.to_string();
                used.insert(id);
            }
            comp_counter += 1;
            if let Some(p) = self.packages.get_mut("BGA-201") {
                p.component_count += 1;
            }
        }

        // 2. Symmetrische 2-Pin-Paare (gegenseitig nächste Nachbarn gleicher Größe)
        let nearest_twin = |aps: &[ApertureItem], used: &HashSet<usize>, id: usize| -> Option<(usize, f64)> {
            let ap = &aps[id];
            let mut best: Option<(usize, f64)> = None;
            for nid in query(ap.cx, ap.cy, 3.2) {
                if nid == id || used.contains(&nid) {
                    continue;
                }
                let n = &aps[nid];
                if (n.width - ap.width).abs() < 0.08 && (n.height - ap.height).abs() < 0.08 {
                    let d = (n.cx - ap.cx).hypot(n.cy - ap.cy);
                    if d >= 0.35 && best.map_or(true, |(_, bd)| d < bd) {
                        best = Some((nid, d));
                    }
                }
            }
            best
        };

        for id in 0..self.apertures.len() {
            if used.contains(&id) {
                continue;
            }
            let Some((bid, dist)) = nearest_twin(&self.apertures, &used, id) else { continue };
            if dist > 3.2 {
                continue;
            }
            if nearest_twin(&self.apertures, &used, bid).map(|(n, _)| n) != Some(id) {
                continue;
            }
            // Pads einer Reihe (Stecker, IC-Seite) haben weitere gleich große Nachbarn im selben
            // Abstand – das ist kein 2-Pin-Chip.
            let in_row = |a: usize, partner: usize| {
                let ap = &self.apertures[a];
                query(ap.cx, ap.cy, dist * 1.2).into_iter().any(|nid| {
                    let n = &self.apertures[nid];
                    nid != a && nid != partner
                        && (n.width - ap.width).abs() < 0.08 && (n.height - ap.height).abs() < 0.08
                        && (n.cx - ap.cx).hypot(n.cy - ap.cy) <= dist * 1.2
                })
            };
            if in_row(id, bid) || in_row(bid, id) {
                continue;
            }
            used.insert(id);
            used.insert(bid);
            let pitch = dist;
            let pkg_name = if pitch < 0.70 {
                "0201"
            } else if pitch < 1.05 {
                "0402"
            } else if pitch < 1.50 {
                "0603"
            } else if pitch < 2.20 {
                "0805"
            } else {
                "1206"
            };
            let (ax, ay, aw, ah) = {
                let a = &self.apertures[id];
                (a.cx, a.cy, r3(a.width), r3(a.height))
            };
            let (bx, by) = (self.apertures[bid].cx, self.apertures[bid].cy);
            // Template liegt horizontal (Pads links/rechts): bei vertikal angeordnetem Paar B/H tauschen
            let (pw, ph) = if (bx - ax).abs() >= (by - ay).abs() { (aw, ah) } else { (ah, aw) };
            let (dx, dy) = (bx - ax, by - ay);
            let (fa, fb) = if dx.abs() >= dy.abs() {
                if dx > 0.0 { ("right", "left") } else { ("left", "right") }
            } else if dy > 0.0 {
                ("up", "down")
            } else {
                ("down", "up")
            };
            if let Some(p) = self.packages.get_mut(pkg_name) {
                let half = r3(pitch / 2.0);
                p.pad_width = pw;
                p.pad_height = ph;
                p.pitch = r3(pitch);
                let round = self.apertures[id].shape_type == "circle";
                p.pads_relative = vec![PadRel::new(1, -half, 0.0, pw, ph, "right").round(round), PadRel::new(2, half, 0.0, pw, ph, "left").round(round)];
                p.outline = Outline { w: r2(pitch + pw + 0.4), h: r2(ph * 1.3) };
                p.component_count += 1;
            }
            for (aid, facing, pin) in [(id, fa, 1), (bid, fb, 2)] {
                let ap = &mut self.apertures[aid];
                ap.facing_dir = facing.into();
                ap.package_name = pkg_name.into();
                ap.component_id = comp_counter.to_string();
                ap.pin_number = pin;
            }
            comp_counter += 1;
        }

        // 3. Cluster-Bildung (BFS, 2.5 mm Nachbarschaft) für ICs / Steckverbinder
        let mut visited: HashSet<usize> = HashSet::new();
        let mut clusters: Vec<Vec<usize>> = Vec::new();
        for id in 0..self.apertures.len() {
            if used.contains(&id) || visited.contains(&id) {
                continue;
            }
            let mut comp = Vec::new();
            let mut queue = VecDeque::from([id]);
            visited.insert(id);
            while let Some(cid) = queue.pop_front() {
                comp.push(cid);
                let (cx, cy) = (self.apertures[cid].cx, self.apertures[cid].cy);
                for nid in query(cx, cy, 2.5) {
                    if visited.contains(&nid) || used.contains(&nid) {
                        continue;
                    }
                    let n = &self.apertures[nid];
                    if (n.cx - cx).hypot(n.cy - cy) <= 2.5 {
                        visited.insert(nid);
                        queue.push_back(nid);
                    }
                }
            }
            clusters.push(comp);
        }

        for comp in clusters {
            let pins = comp.len();
            let (pkg_name, category) = match pins {
                8 => ("SOIC-8".to_string(), "ICs & Transistors"),
                16 => ("SOIC-16".to_string(), "ICs & Transistors"),
                24 | 25 => ("QFN-24".to_string(), "ICs & Transistors"),
                32 | 33 => ("QFN-32".to_string(), "ICs & Transistors"),
                48 | 49 => ("QFN-48".to_string(), "ICs & Transistors"),
                n if n >= 3 => (format!("Generic_{n}Pin"), if matches!(n, 10 | 12 | 14 | 20) { "Connectors" } else { "ICs & Transistors" }),
                _ => continue,
            };
            let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
            for &i in &comp {
                let a = &self.apertures[i];
                x0 = x0.min(a.cx - a.width / 2.0);
                x1 = x1.max(a.cx + a.width / 2.0);
                y0 = y0.min(a.cy - a.height / 2.0);
                y1 = y1.max(a.cy + a.height / 2.0);
            }
            let (mcx, mcy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let (cw, ch) = ((x1 - x0).max(0.5), (y1 - y0).max(0.5));

            let mut sorted = comp.clone();
            sorted.sort_by(|&a, &b| {
                let (pa, pb) = (&self.apertures[a], &self.apertures[b]);
                (-r2(pa.cy)).partial_cmp(&-r2(pb.cy)).unwrap_or(std::cmp::Ordering::Equal).then(r2(pa.cx).partial_cmp(&r2(pb.cx)).unwrap_or(std::cmp::Ordering::Equal))
            });

            let mut pads_rel = Vec::with_capacity(pins);
            for (i, &aid) in sorted.iter().enumerate() {
                let ap = &mut self.apertures[aid];
                let (adx, ady) = (mcx - ap.cx, mcy - ap.cy);
                ap.facing_dir = if adx.abs() < 0.05 && ady.abs() < 0.05 {
                    "center"
                } else if adx.abs() >= ady.abs() {
                    if adx > 0.0 { "right" } else { "left" }
                } else if ady > 0.0 {
                    "up"
                } else {
                    "down"
                }
                .into();
                let mut pr = PadRel::new(i as i64 + 1, r3(ap.cx - mcx), r3(ap.cy - mcy), r3(ap.width), r3(ap.height), &ap.facing_dir).round(ap.shape_type == "circle");
                pr.is_thermal = ap.width >= 1.5 && ap.height >= 1.5 && pins > 4;
                pads_rel.push(pr);
            }

            let outline = Outline { w: r2(cw + 0.6), h: r2(ch + 0.6) };
            let rel: Vec<(f64, f64, f64, f64)> = pads_rel.iter().map(|p| (p.cx, p.cy, p.w, p.h)).collect();
            let real_pitch = (0..rel.len()).filter_map(|i| super::neighbor_pitch(&rel, i)).fold(f64::INFINITY, f64::min);
            match self.packages.get_mut(&pkg_name) {
                Some(p) => {
                    if real_pitch.is_finite() {
                        p.pitch = r3(real_pitch);
                    }
                    p.pads_relative = pads_rel;
                    p.outline = outline;
                    p.component_count += 1;
                }
                None => {
                    let avg_w = comp.iter().map(|&i| self.apertures[i].width).sum::<f64>() / pins as f64;
                    let avg_h = comp.iter().map(|&i| self.apertures[i].height).sum::<f64>() / pins as f64;
                    let pitch = if real_pitch.is_finite() { real_pitch } else { 0.8 };
                    let mut p = PackageTemplate::new(&pkg_name, pins, r3(pitch), r2(avg_w), r2(avg_h));
                    p.category = category.into();
                    p.rule = PackageRule::with("package_group", "Handle Stencil Creation by Package Group");
                    p.outline = outline;
                    p.pads_relative = pads_rel;
                    p.component_count = 1;
                    self.packages.insert(pkg_name.clone(), p);
                }
            }
            for (i, &aid) in sorted.iter().enumerate() {
                let ap = &mut self.apertures[aid];
                ap.package_name = pkg_name.clone();
                ap.component_id = comp_counter.to_string();
                ap.pin_number = i as i64 + 1;
                used.insert(aid);
            }
            comp_counter += 1;
        }

        // 4. Einzelpads
        for id in 0..self.apertures.len() {
            if used.contains(&id) {
                continue;
            }
            let (w, h) = (self.apertures[id].width, self.apertures[id].height);
            let round = self.apertures[id].shape_type == "circle";
            let grp = format!("Generic_{:.2}x{:.2}", w, h);
            {
                let ap = &mut self.apertures[id];
                ap.package_name = grp.clone();
                ap.component_id = comp_counter.to_string();
                ap.pin_number = 1;
                ap.facing_dir = "center".into();
            }
            comp_counter += 1;
            let p = self.packages.entry(grp.clone()).or_insert_with(|| {
                let mut p = PackageTemplate::new(&grp, 1, 0.0, w, h);
                p.category = "Other".into();
                p.rule = PackageRule::with("package_group", "Handle Stencil Creation by Package Group");
                p.component_count = 0;
                p.pads_relative = vec![PadRel::new(1, 0.0, 0.0, w, h, "center").round(round)];
                p
            });
            p.component_count += 1;
        }
    }

    /// Automatische Bauteil-Klassifizierung und Regelzuweisung (non-destruktiv).
    pub fn auto_classify_and_apply_rules(&mut self) -> Value {
        let (mut chips, mut soic, mut qfn, mut bga, mut tht, mut other, mut modified) = (0, 0, 0, 0, 0, 0, 0usize);
        let mut details = Vec::new();

        let names: Vec<String> = self.packages.keys().cloned().collect();
        for name in &names {
            let pkg = self.packages.get_mut(name).expect("Package vorhanden");
            let (pins, pitch, pw, ph) = (pkg.pin_count, pkg.pitch, pkg.pad_width, pkg.pad_height);
            let upper = name.to_uppercase();
            let has_thermal = pkg.pads_relative.iter().any(|p| p.is_thermal);
            let big_center = pkg.pads_relative.iter().any(|p| p.is_thermal || (p.w >= 1.5 && p.h >= 1.5 && p.pin > 4));

            let (class, rule): (&str, Option<PackageRule>) = if pins == 2 && pitch < 3.2 {
                chips += pkg.component_count;
                if pitch < 0.8 {
                    ("SMD Chip", Some(PackageRule { name: "Simple Rectangle (Fine-Pitch Chip)".into(), undersize_w: 0.015, undersize_h: 0.015, ..Default::default() }))
                } else {
                    ("SMD Chip", Some(PackageRule { name: "HouseBuilder (Anti-Tombstone)".into(), rule_type: "house_builder".into(), house_cut_pct: 30.0, ..Default::default() }))
                }
            } else if (6..=32).contains(&pins) && (0.45..=1.35).contains(&pitch) && !has_thermal {
                soic += pkg.component_count;
                ("SOIC / SOP", Some(PackageRule { name: "Länglich & Schmal (Pitch-Schutz)".into(), rule_type: "elongated_pitch".into(), ..Default::default() }))
            } else if big_center || upper.contains("QFN") || upper.contains("DFN") {
                qfn += pkg.component_count;
                for p in &pkg.pads_relative {
                    if p.is_thermal || (p.w >= 1.5 && p.h >= 1.5) {
                        let grid = if p.w.max(p.h) >= 3.0 { 3 } else { 2 };
                        let ov = json!({
                            "rule_type": "array_slicing",
                            "rotate_bridges_45": true,
                            "bridge_count_x": grid,
                            "bridge_count_y": grid,
                            "bridge_width": 0.30,
                            "w": p.w,
                            "h": p.h,
                        });
                        if let Value::Object(m) = ov {
                            pkg.pad_overrides.insert(p.pin, m);
                        }
                    }
                }
                ("QFN / DFN", Some(PackageRule { name: "QFN Peripheral (Pitch-Schutz)".into(), rule_type: "elongated_pitch".into(), pitch_reduction_pct: 15.0, ..Default::default() }))
            } else if pins >= 16 && (upper.contains("BGA") || (pw == ph && pw <= 0.60 && pitch <= 1.0)) {
                bga += pkg.component_count;
                ("BGA / CSP", Some(PackageRule { name: "BGA Release Reduction".into(), undersize_w: 0.02, undersize_h: 0.02, ..Default::default() }))
            } else if upper.contains("THT") || upper.contains("HEADER") || upper.contains("PIN") {
                tht += pkg.component_count;
                ("THT / Through-Hole", None)
            } else {
                other += pkg.component_count;
                ("Other", None)
            };

            if let Some(r) = rule {
                details.push(json!({"package": name, "class": class, "rule": r.name, "count": pkg.component_count}));
                pkg.rule = r;
                pkg.category = match class {
                    "SMD Chip" => "SMD Passives",
                    c if c.contains("SOIC") || c.contains("QFN") => "ICs & Transistors",
                    c if c.contains("BGA") => "BGA & Arrays",
                    _ => "Other",
                }
                .into();
            }
        }

        for name in &names {
            let rule = self.packages[name].rule.to_json();
            if let Value::Object(m) = rule {
                modified += self.apply_package_rule(name, &m);
            }
        }

        json!({
            "chips": chips,
            "soic": soic,
            "qfn": qfn,
            "bga": bga,
            "tht": tht,
            "other": other,
            "modified_apertures": modified,
            "details": details,
        })
    }
}
