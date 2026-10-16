use crate::geometry::{
    calc_array_slicing, calc_chamfer, calc_corner_radius, calc_elongated_pitch, calc_house_builder,
    calc_inverted_house_builder, calc_rectangle, rotate_polygon, ChamferMode,
};
use crate::model::{Aperture, Pad, RuleConfig, StencilWorkspace};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentClass {
    Chip,
    Soic,
    Qfn,
    Bga,
    Tht,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationStats {
    pub chip_count: usize,
    pub soic_count: usize,
    pub qfn_count: usize,
    pub bga_count: usize,
    pub tht_count: usize,
    pub other_count: usize,
    pub modified_apertures: usize,
}

/// Ermittelt die gedrehte Ausrichtungsrichtung ("left", "right", "up", "down") basierend auf Bauteil-Rotation.
pub fn rotate_facing_dir(facing: &str, rot_deg: f64) -> String {
    let steps = (((rot_deg % 360.0 + 360.0) % 360.0) / 90.0).round() as i32 % 4;
    let dirs = ["down", "left", "up", "right"];
    let idx = match facing.to_lowercase().as_str() {
        "left" => 1,
        "up" => 2,
        "right" => 3,
        _ => 0, // down
    };
    dirs[(idx + steps as usize) % 4].to_string()
}

/// Generiert Schablonengeometrien für ein Pad gemäß einer Regel-Konfiguration.
pub fn apply_rule_to_pad(pad: &Pad, rule: &RuleConfig, rot_deg: f64) -> (Vec<crate::model::Point>, f64, String) {
    let facing = rotate_facing_dir(&pad.facing_dir, rot_deg);

    let (pts, area, shape_name) = match rule.rule_type.as_str() {
        "house_builder" => {
            let (p, a) = calc_house_builder(pad.cx, pad.cy, pad.width, pad.height, rule.house_cut_pct, &facing);
            (p, a, "house_builder".to_string())
        }
        "inverted_house_builder" => {
            let (p, a) = calc_inverted_house_builder(pad.cx, pad.cy, pad.width, pad.height, rule.house_cut_pct, &facing);
            (p, a, "inverted_house_builder".to_string())
        }
        "elongated_pitch" => {
            let (p, a) = calc_elongated_pitch(pad.cx, pad.cy, pad.width, pad.height, rule.pitch_reduction_pct, rule.corner_radius);
            (p, a, "elongated_pitch".to_string())
        }
        "chamfer" => {
            let mode = match rule.chamfer_mode.to_lowercase().as_str() {
                "pin1" => ChamferMode::Pin1,
                "toe" => ChamferMode::Toe,
                "heel" => ChamferMode::Heel,
                _ => ChamferMode::All,
            };
            let (p, a) = calc_chamfer(pad.cx, pad.cy, pad.width, pad.height, rule.chamfer_size, mode, &facing);
            (p, a, "chamfer".to_string())
        }
        "corner_radius" => {
            let (p, a) = calc_corner_radius(pad.cx, pad.cy, pad.width, pad.height, rule.corner_radius, 3);
            (p, a, "corner_radius".to_string())
        }
        _ => {
            let (p, a) = calc_rectangle(pad.cx, pad.cy, pad.width, pad.height, rule.undersize_w, rule.undersize_h);
            (p, a, "simple_rect".to_string())
        }
    };

    // Nicht-orthogonale oder zusätzliche Rotationen anwenden
    let rotated_pts = if rot_deg % 90.0 != 0.0 {
        rotate_polygon(&pts, pad.cx, pad.cy, rot_deg)
    } else {
        pts
    };

    (rotated_pts, area, shape_name)
}

/// Heuristische Klassifizierung von Bauteilen und automatische Zuweisung optimaler DFM-Regeln.
pub fn classify_and_apply_rules(workspace: &mut StencilWorkspace) -> ClassificationStats {
    let mut stats = ClassificationStats {
        chip_count: 0,
        soic_count: 0,
        qfn_count: 0,
        bga_count: 0,
        tht_count: 0,
        other_count: 0,
        modified_apertures: 0,
    };

    // 1. Gruppiere Pads pro Package und RefDes
    let mut pkg_pads: HashMap<String, Vec<&Pad>> = HashMap::new();
    let comp_map: HashMap<String, f64> = workspace.components.iter().map(|c| (c.refdes.clone(), c.rotation)).collect();

    for pad in &workspace.working_top.pads {
        pkg_pads.entry(pad.package_name.clone()).or_default().push(pad);
    }

    let mut pkg_rules: HashMap<String, (ComponentClass, RuleConfig)> = HashMap::new();

    for (pkg_name, pads) in &pkg_pads {
        let pin_count = pads.len();
        let has_thermal = pads.iter().any(|p| p.is_thermal || (p.width >= 2.0 && p.height >= 2.0));
        let name_upper = pkg_name.to_uppercase();

        let mut class = ComponentClass::Other;
        let mut rule = RuleConfig::default();

        // 1. Heuristik: 2 Pins, symmetrisch, Abstand < 3.5 mm -> CHIP (0201 bis 1206)
        if pin_count == 2 {
            let d = ((pads[0].cx - pads[1].cx).powi(2) + (pads[0].cy - pads[1].cy).powi(2)).sqrt();
            if d < 3.5 {
                class = ComponentClass::Chip;
                stats.chip_count += 1;
                // HouseBuilder (Reduktion 10 %, Giebel nach innen zur Vermeidung von Solder Beads)
                rule = RuleConfig {
                    rule_type: "house_builder".to_string(),
                    undersize_w: 0.02,
                    undersize_h: 0.02,
                    house_cut_pct: 10.0,
                    ..Default::default()
                };
            }
        }
        // 2. Heuristik: QFN / DFN mit zentralem Exposed Thermal Pad
        else if has_thermal || name_upper.contains("QFN") || name_upper.contains("DFN") {
            class = ComponentClass::Qfn;
            stats.qfn_count += 1;
            // Standardmäßig periphere Pins mit Pitch-Schutz versehen
            rule = RuleConfig {
                rule_type: "elongated_pitch".to_string(),
                pitch_reduction_pct: 15.0,
                corner_radius: 0.05,
                ..Default::default()
            };
        }
        // 3. Heuristik: SOIC / SOP (2 parallele Reihen, Pitch 0.5 - 1.27 mm)
        else if (6..=32).contains(&pin_count) && (name_upper.contains("SOIC") || name_upper.contains("SOP") || name_upper.contains("TSSOP")) {
            class = ComponentClass::Soic;
            stats.soic_count += 1;
            rule = RuleConfig {
                rule_type: "elongated_pitch".to_string(),
                pitch_reduction_pct: 15.0,
                corner_radius: 0.05,
                ..Default::default()
            };
        }
        // 4. Heuristik: BGA (Matrix runde Pins)
        else if pin_count >= 16 && (name_upper.contains("BGA") || name_upper.contains("CSP")) {
            class = ComponentClass::Bga;
            stats.bga_count += 1;
            rule = RuleConfig {
                rule_type: "simple_rect".to_string(),
                undersize_w: 0.02,
                undersize_h: 0.02,
                ..Default::default()
            };
        }
        // 5. Heuristik: THT
        else if name_upper.contains("THT") || name_upper.contains("HEADER") || name_upper.contains("DIP") {
            class = ComponentClass::Tht;
            stats.tht_count += 1;
        } else {
            stats.other_count += 1;
        }

        pkg_rules.insert(pkg_name.clone(), (class, rule));
    }

    // 2. Wende Regeln auf Aperturen im Schablonen-Zwischenspeicher an
    let pad_map: HashMap<usize, &Pad> = workspace.working_top.pads.iter().map(|p| (p.id, p)).collect();

    let mut new_apertures = Vec::with_capacity(workspace.working_top.apertures.len());

    for mut ap in workspace.working_top.apertures.drain(..) {
        if ap.manual_override || ap.is_custom {
            new_apertures.push(ap);
            continue;
        }

        if let Some(pad) = pad_map.get(&ap.id).copied() {
            let rot = comp_map.get(&pad.refdes).copied().unwrap_or(0.0);
            ap.rotation = rot;
            ap.component_refdes = pad.refdes.clone();

            if let Some((class, rule)) = pkg_rules.get(&ap.package_name) {
                // Sonderfall: Thermal Pad eines QFN erhält 45°-Array Slicing
                if (*class == ComponentClass::Qfn) && (pad.is_thermal || (pad.width >= 2.0 && pad.height >= 2.0)) {
                    let slices = calc_array_slicing(pad.cx, pad.cy, pad.width, pad.height, 2, 2, 0.20, true);
                    for (idx, (s_pts, s_area)) in slices.into_iter().enumerate() {
                        let sub_ap = Aperture {
                            id: ap.id * 1000 + idx,
                            pad_id: Some(pad.id),
                            package_name: ap.package_name.clone(),
                            component_refdes: pad.refdes.clone(),
                            pin: ap.pin,
                            cx: pad.cx,
                            cy: pad.cy,
                            width: pad.width,
                            height: pad.height,
                            facing_dir: pad.facing_dir.clone(),
                            rotation: rot,
                            points: s_pts,
                            area: s_area,
                            shape_type: "array_45".to_string(),
                            is_modified: true,
                            is_custom: false,
                            manual_override: false,
                        };
                        new_apertures.push(sub_ap);
                    }
                    stats.modified_apertures += 1;
                    continue;
                }

                // Standard-Regel für Bauteilklasse anwenden
                let (pts, area, shape_name) = apply_rule_to_pad(pad, rule, rot);
                ap.points = pts;
                ap.area = area;
                ap.shape_type = shape_name;
                ap.is_modified = true;
                stats.modified_apertures += 1;
            }
        }
        new_apertures.push(ap);
    }

    workspace.working_top.apertures = new_apertures;
    stats
}

