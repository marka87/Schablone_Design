pub mod dfm;
pub mod exporter;
pub mod geometry;
pub mod model;
pub mod parser;

use dfm::{
    analyze_ipc_parallel, calculate_pip as calc_pip_core, check_stencil_bridges,
    BridgeViolation, IpcResult, IpcStatus, PipParams, PipResult,
};
use exporter::StencilSide;
use geometry::{
    calc_array_slicing, calc_chamfer, calc_corner_radius, calc_house_builder,
    calc_inverted_house_builder, calc_rectangle, ChamferMode,
};
use model::{
    classifier, presets, Aperture, ClassificationStats, Pad, RuleConfig, RuleDatabase,
    StencilWorkspace, StepZone,
};
use parser::WorkspaceSummary;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    pub workspace: Mutex<StencilWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleParams {
    pub rule_type: String,
    pub pad_width: f64,
    pub pad_height: f64,
    pub cx: f64,
    pub cy: f64,
    pub undersize_w: Option<f64>,
    pub undersize_h: Option<f64>,
    pub corner_radius: Option<f64>,
    pub house_cut_pct: Option<f64>,
    pub bridge_nx: Option<usize>,
    pub bridge_my: Option<usize>,
    pub bridge_width: Option<f64>,
    pub rotate_45: Option<bool>,
    pub chamfer_size: Option<f64>,
    pub chamfer_mode: Option<String>,
    pub facing_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfmSummary {
    pub total_apertures: usize,
    pub ok_count: usize,
    pub warn_count: usize,
    pub crit_count: usize,
    pub bridge_violations_count: usize,
    pub grain_warn_count: usize,
    pub min_area_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DfmReport {
    pub summary: DfmSummary,
    pub ipc_results: Vec<IpcResult>,
    pub bridge_violations: Vec<BridgeViolation>,
}

// -------------------------------------------------------------
// TAURI COMMANDS
// -------------------------------------------------------------

/// 1. Wendet eine Schablonenregel auf eine Pad-Geometrie an und gibt Apertur-Polygone zurück.
#[tauri::command]
fn apply_package_rule(params: RuleParams) -> Result<Vec<Aperture>, String> {
    let uw = params.undersize_w.unwrap_or(0.0);
    let uh = params.undersize_h.unwrap_or(0.0);
    let facing = params.facing_dir.as_deref().unwrap_or("down");

    let rule = params.rule_type.to_lowercase();
    let mut apertures = Vec::new();

    match rule.as_str() {
        "house_builder" => {
            let cut = params.house_cut_pct.unwrap_or(30.0);
            let (pts, area) = calc_house_builder(params.cx, params.cy, params.pad_width, params.pad_height, cut, facing);
            apertures.push(Aperture {
                id: 1,
                pad_id: None,
                package_name: "custom".into(),
                pin: 1,
                cx: params.cx,
                cy: params.cy,
                width: params.pad_width,
                height: params.pad_height,
                points: pts,
                area,
                shape_type: "house_builder".into(),
                is_modified: true,
                manual_override: false,
                ..Default::default()
            });
        }
        "inverted_house_builder" => {
            let cut = params.house_cut_pct.unwrap_or(30.0);
            let (pts, area) = calc_inverted_house_builder(params.cx, params.cy, params.pad_width, params.pad_height, cut, facing);
            apertures.push(Aperture {
                id: 1,
                pad_id: None,
                package_name: "custom".into(),
                pin: 1,
                cx: params.cx,
                cy: params.cy,
                width: params.pad_width,
                height: params.pad_height,
                points: pts,
                area,
                shape_type: "inverted_house_builder".into(),
                is_modified: true,
                manual_override: false,
                ..Default::default()
            });
        }
        "chamfer" => {
            let c_size = params.chamfer_size.unwrap_or(0.10);
            let mode = match params.chamfer_mode.as_deref() {
                Some("pin1") => ChamferMode::Pin1,
                Some("toe") => ChamferMode::Toe,
                Some("heel") => ChamferMode::Heel,
                _ => ChamferMode::All,
            };
            let (pts, area) = calc_chamfer(params.cx, params.cy, params.pad_width, params.pad_height, c_size, mode, facing);
            apertures.push(Aperture {
                id: 1,
                pad_id: None,
                package_name: "custom".into(),
                pin: 1,
                cx: params.cx,
                cy: params.cy,
                width: params.pad_width,
                height: params.pad_height,
                points: pts,
                area,
                shape_type: "chamfer".into(),
                is_modified: true,
                manual_override: false,
                ..Default::default()
            });
        }
        "corner_radius" => {
            let r = params.corner_radius.unwrap_or(0.05);
            let (pts, area) = calc_corner_radius(params.cx, params.cy, params.pad_width, params.pad_height, r, 3);
            apertures.push(Aperture {
                id: 1,
                pad_id: None,
                package_name: "custom".into(),
                pin: 1,
                cx: params.cx,
                cy: params.cy,
                width: params.pad_width,
                height: params.pad_height,
                points: pts,
                area,
                shape_type: "corner_radius".into(),
                is_modified: true,
                manual_override: false,
                ..Default::default()
            });
        }
        "array_slicing" => {
            let nx = params.bridge_nx.unwrap_or(2);
            let my = params.bridge_my.unwrap_or(2);
            let bw = params.bridge_width.unwrap_or(0.30);
            let rot45 = params.rotate_45.unwrap_or(false);

            let slices = calc_array_slicing(params.cx, params.cy, params.pad_width, params.pad_height, nx, my, bw, rot45);
            for (idx, (pts, area)) in slices.into_iter().enumerate() {
                apertures.push(Aperture {
                    id: idx + 1,
                    pad_id: None,
                    package_name: "thermal_pad".into(),
                    pin: idx + 1,
                    cx: params.cx,
                    cy: params.cy,
                    width: params.pad_width,
                    height: params.pad_height,
                    points: pts,
                    area,
                    shape_type: if rot45 { "array_45".into() } else { "array_rect".into() },
                    is_modified: true,
                    manual_override: false,
                    ..Default::default()
                });
            }
        }
        _ => {
            // Default: Simple Rectangle mit Undersize
            let (pts, area) = calc_rectangle(params.cx, params.cy, params.pad_width, params.pad_height, uw, uh);
            apertures.push(Aperture {
                id: 1,
                pad_id: None,
                package_name: "custom".into(),
                pin: 1,
                cx: params.cx,
                cy: params.cy,
                width: params.pad_width - 2.0 * uw,
                height: params.pad_height - 2.0 * uh,
                points: pts,
                area,
                shape_type: "simple_rect".into(),
                is_modified: true,
                manual_override: false,
                ..Default::default()
            });
        }
    }

    Ok(apertures)
}

/// 2. Führt die vollständige DFM-Analyse (IPC-7525 + Web Check) auf dem aktuellen Schablonen-Zwischenspeicher aus.
#[tauri::command]
fn run_dfm_analysis(
    stencil_thickness: f64,
    paste_type: Option<String>,
    min_bridge: Option<f64>,
    custom_apertures: Option<Vec<Aperture>>,
    state: State<'_, AppState>,
) -> Result<DfmReport, String> {
    let p_type = paste_type.as_deref().unwrap_or("type4");
    let m_bridge = min_bridge.unwrap_or(0.11);

    // Entweder benutzerdefinierte Aperturen prüfen oder aus dem aktiven Arbeits-Layer laden
    let apertures = match custom_apertures {
        Some(aps) => aps,
        None => {
            let ws = state.workspace.lock().map_err(|e| e.to_string())?;
            ws.working_top.apertures.clone()
        }
    };

    if apertures.is_empty() {
        return Ok(DfmReport {
            summary: DfmSummary {
                total_apertures: 0,
                ok_count: 0,
                warn_count: 0,
                crit_count: 0,
                bridge_violations_count: 0,
                grain_warn_count: 0,
                min_area_ratio: 0.0,
            },
            ipc_results: Vec::new(),
            bridge_violations: Vec::new(),
        });
    }

    // Parallele IPC-Berechnung via Rayon
    let ipc_results = analyze_ipc_parallel(&apertures, stencil_thickness, p_type);

    // R-Tree Stegbreiten-Prüfung
    let bridge_violations = check_stencil_bridges(&apertures, m_bridge);

    let mut ok_c = 0;
    let mut warn_c = 0;
    let mut crit_c = 0;
    let mut grain_c = 0;
    let mut min_ar = f64::INFINITY;

    for r in &ipc_results {
        match r.status {
            IpcStatus::Ok => ok_c += 1,
            IpcStatus::Warn => warn_c += 1,
            IpcStatus::Crit => crit_c += 1,
        }
        if r.grain_warn {
            grain_c += 1;
        }
        if r.area_ratio < min_ar {
            min_ar = r.area_ratio;
        }
    }

    let summary = DfmSummary {
        total_apertures: apertures.len(),
        ok_count: ok_c,
        warn_count: warn_c,
        crit_count: crit_c,
        bridge_violations_count: bridge_violations.len(),
        grain_warn_count: grain_c,
        min_area_ratio: if min_ar.is_infinite() { 0.0 } else { min_ar },
    };

    Ok(DfmReport {
        summary,
        ipc_results,
        bridge_violations,
    })
}

/// 3. Pin-in-Paste Berechnungsmodul für Through-Hole Reflow
#[tauri::command]
fn calculate_pip(params: PipParams) -> Result<PipResult, String> {
    Ok(calc_pip_core(&params))
}

/// 4. ODB++ Archiv (.zip, .tgz) oder Ordner einlesen und automatisch vor-klassifizieren
#[tauri::command]
fn load_odb_file(file_path: String, state: State<'_, AppState>) -> Result<WorkspaceSummary, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let summary = parser::parse_odb_archive(&file_path, &mut ws)?;
    // Automatische Bauteilerkennung und Regelvorbelegung direkt beim Laden
    let _ = classifier::classify_and_apply_rules(&mut ws);
    Ok(summary)
}

/// Öffnet einen nativen Datei-Auswahldialog und gibt den absoluten Dateipfad zurück
#[tauri::command]
fn select_file_dialog() -> Option<String> {
    let file = rfd::FileDialog::new()
        .add_filter("ODB++ / Gerber Archive", &["zip", "tgz", "tar.gz", "tar"])
        .add_filter("Gerber-Dateien (*.ger, *.gbr, *.pho)", &["ger", "gbr", "pho"])
        .add_filter("Alle Dateien", &["*"])
        .set_title("ODB++ / Gerber Archiv auswählen")
        .pick_file();
    file.map(|p| p.to_string_lossy().to_string())
}

/// Öffnet einen nativen Ordner-Auswahldialog und gibt den absoluten Pfad zurück
#[tauri::command]
fn select_folder_dialog() -> Option<String> {
    let folder = rfd::FileDialog::new()
        .set_title("ODB++ Ordner auswählen")
        .pick_folder();
    folder.map(|p| p.to_string_lossy().to_string())
}

/// Liest ODB++ Archivdaten direkt als Byte-Array (für Drag & Drop & Upload ohne Pfad-Fehler)
#[tauri::command]
fn load_odb_bytes(
    file_name: String,
    data: Vec<u8>,
    state: State<'_, AppState>,
) -> Result<WorkspaceSummary, String> {
    let temp_dir = std::env::temp_dir();
    let sanitized_name = std::path::Path::new(&file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("import.zip");
    let temp_path = temp_dir.join(format!("stencilcraft_{}_{}", std::process::id(), sanitized_name));

    std::fs::write(&temp_path, &data).map_err(|e| format!("Fehler beim Schreiben der temporären Importdatei: {}", e))?;

    let res = load_odb_file(temp_path.to_string_lossy().to_string(), state);
    let _ = std::fs::remove_file(&temp_path);
    res
}

/// 5. Modifizierte Schablonenöffnungen als normgerechte RS-274X Gerber-Datei exportieren
#[tauri::command]
fn export_stencil_gerber(
    target_dir: String,
    side: StencilSide,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let (layer, zones) = match side {
        StencilSide::Top => (&ws.working_top, &ws.top_zones),
        StencilSide::Bot => (&ws.working_bot, &ws.bot_zones),
    };
    let res = exporter::export_stencil_gerber(layer, zones, ws.base_thickness, std::path::Path::new(&target_dir), side)?;
    let zones_info = res.zones_file.map(|z| format!(" + Zonen: {}", z)).unwrap_or_default();
    Ok(format!(
        "Gerber erfolgreich exportiert: {} ({} Öffnungen){}",
        res.stencil_file, res.aperture_count, zones_info
    ))
}

/// 6. Aperturen des Arbeits-Zwischenspeichers abfragen
#[tauri::command]
fn get_workspace_apertures(side: Option<String>, state: State<'_, AppState>) -> Result<Vec<Aperture>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let layer = if is_bot { &ws.working_bot } else { &ws.working_top };
    Ok(layer.apertures.clone())
}

/// 7. Aktualisiert alle Aperturen eines Packages im Arbeits-Zwischenspeicher unter Beachtung der Drehung
#[tauri::command]
fn update_package_rule(
    package_name: String,
    rule_config: RuleConfig,
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<Aperture>, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let comp_map: HashMap<String, f64> = ws.components.iter().map(|c| (c.refdes.clone(), c.rotation)).collect();
    let layer = if is_bot { &mut ws.working_bot } else { &mut ws.working_top };

    let pad_map: HashMap<usize, Pad> = layer.pads.iter().map(|p| (p.id, p.clone())).collect();

    let mut modified = Vec::new();
    for ap in &mut layer.apertures {
        if ap.package_name == package_name && !ap.is_custom && !ap.manual_override {
            if let Some(pad) = pad_map.get(&ap.id) {
                let rot = comp_map.get(&pad.refdes).copied().unwrap_or(ap.rotation);
                let (pts, area, shape_name) = classifier::apply_rule_to_pad(pad, &rule_config, rot);
                ap.points = pts;
                ap.area = area;
                ap.shape_type = shape_name;
                ap.is_modified = true;
                ap.rotation = rot;
                modified.push(ap.clone());
            }
        }
    }

    Ok(modified)
}

/// 8. Übersteuert gezielt einen einzelnen Pin eines Bauteils
#[tauri::command]
fn set_pin_override(
    component_refdes: String,
    pin_number: usize,
    override_config: Option<RuleConfig>,
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<Aperture, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let comp_map: HashMap<String, f64> = ws.components.iter().map(|c| (c.refdes.clone(), c.rotation)).collect();
    let layer = if is_bot { &mut ws.working_bot } else { &mut ws.working_top };

    let pad_map: HashMap<usize, Pad> = layer.pads.iter().map(|p| (p.id, p.clone())).collect();

    for ap in &mut layer.apertures {
        if ap.component_refdes == component_refdes && ap.pin == pin_number {
            if let Some(pad) = pad_map.get(&ap.id) {
                let rot = comp_map.get(&pad.refdes).copied().unwrap_or(ap.rotation);
                match override_config {
                    Some(rule) => {
                        let (pts, area, shape_name) = classifier::apply_rule_to_pad(pad, &rule, rot);
                        ap.points = pts;
                        ap.area = area;
                        ap.shape_type = shape_name;
                        ap.is_modified = true;
                        ap.is_custom = true;
                        ap.manual_override = true;
                        ap.rotation = rot;
                    }
                    None => {
                        let (pts, area) = geometry::calc_rectangle(pad.cx, pad.cy, pad.width, pad.height, 0.025, 0.025);
                        ap.points = pts;
                        ap.area = area;
                        ap.shape_type = "simple_rect".to_string();
                        ap.is_modified = false;
                        ap.is_custom = false;
                        ap.manual_override = false;
                    }
                }
                return Ok(ap.clone());
            }
        }
    }

    Err(format!("Pin {} an Bauteil {} nicht gefunden", pin_number, component_refdes))
}

/// 9. Manuelle Auslösung der Auto-Klassifizierung
#[tauri::command]
fn auto_classify_workspace(state: State<'_, AppState>) -> Result<ClassificationStats, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    Ok(classifier::classify_and_apply_rules(&mut ws))
}

/// 10. Generiert ein druckfertiges HTML DFM- und Fertigungs-Begleitblatt
#[tauri::command]
fn generate_dfm_report(
    target_path: String,
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let side_str = if is_bot { "BOT" } else { "TOP" };
    let layer = if is_bot { &ws.working_bot } else { &ws.working_top };

    let ipc_results = analyze_ipc_parallel(&layer.apertures, ws.base_thickness, "type4");
    let bridge_violations = check_stencil_bridges(&layer.apertures, 0.11);

    let mut ok_c = 0;
    let mut warn_c = 0;
    let mut crit_c = 0;
    let mut grain_c = 0;
    let mut min_ar = f64::INFINITY;

    for r in &ipc_results {
        match r.status {
            IpcStatus::Ok => ok_c += 1,
            IpcStatus::Warn => warn_c += 1,
            IpcStatus::Crit => crit_c += 1,
        }
        if r.grain_warn {
            grain_c += 1;
        }
        if r.area_ratio < min_ar {
            min_ar = r.area_ratio;
        }
    }

    let summary = DfmSummary {
        total_apertures: layer.apertures.len(),
        ok_count: ok_c,
        warn_count: warn_c,
        crit_count: crit_c,
        bridge_violations_count: bridge_violations.len(),
        grain_warn_count: grain_c,
        min_area_ratio: if min_ar.is_infinite() { 0.0 } else { min_ar },
    };

    let dfm = DfmReport {
        summary,
        ipc_results,
        bridge_violations,
    };

    let p = std::path::Path::new(&target_path);
    dfm::write_dfm_report_file(&ws, &dfm, "SMT_Job", side_str, p)
}

/// 11. Stufenschablonen-Zone hinzufügen
#[tauri::command]
fn add_step_zone(
    zone: StepZone,
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<StepZone>, String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let zones = if is_bot { &mut ws.bot_zones } else { &mut ws.top_zones };
    zones.retain(|z| z.id != zone.id);
    zones.push(zone);
    Ok(zones.clone())
}

/// 12. Stufenschablonen-Zonen leeren
#[tauri::command]
fn clear_step_zones(
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    if is_bot {
        ws.bot_zones.clear();
    } else {
        ws.top_zones.clear();
    }
    Ok(())
}

/// 13. Stufenschablonen-Zonen abfragen
#[tauri::command]
fn get_step_zones(
    side: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<StepZone>, String> {
    let ws = state.workspace.lock().map_err(|e| e.to_string())?;
    let is_bot = side.as_deref().map(|s| s.eq_ignore_ascii_case("bot")).unwrap_or(false);
    let zones = if is_bot { &ws.bot_zones } else { &ws.top_zones };
    Ok(zones.clone())
}

/// 14. Kunden-Bibliothek speichern
#[tauri::command]
fn save_customer_library(name: String, rules: RuleDatabase) -> Result<String, String> {
    let p = presets::save_customer_library(&name, &rules)?;
    Ok(p.to_string_lossy().to_string())
}

/// 15. Kunden-Bibliothek laden
#[tauri::command]
fn load_customer_library(name: String) -> Result<RuleDatabase, String> {
    presets::load_customer_library(&name)
}

/// 16. Kunden-Bibliotheken auflisten
#[tauri::command]
fn list_customer_libraries() -> Result<Vec<String>, String> {
    Ok(presets::list_customer_libraries())
}

/// 17. Öffnet eine Datei oder URL im Standard-Systemprogramm (z. B. HTML-Bericht im Standardbrowser)
#[tauri::command]
fn open_path_in_system(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &path])
            .spawn()
            .map_err(|e| format!("Fehler beim Öffnen: {}", e))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Fehler beim Öffnen: {}", e))?;
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            workspace: Mutex::new(StencilWorkspace::new()),
        })
        .invoke_handler(tauri::generate_handler![
            apply_package_rule,
            run_dfm_analysis,
            calculate_pip,
            load_odb_file,
            export_stencil_gerber,
            get_workspace_apertures,
            update_package_rule,
            set_pin_override,
            auto_classify_workspace,
            generate_dfm_report,
            add_step_zone,
            clear_step_zones,
            get_step_zones,
            save_customer_library,
            load_customer_library,
            list_customer_libraries,
            open_path_in_system,
            select_file_dialog,
            select_folder_dialog,
            load_odb_bytes
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der SMT StencilCraft Desktop Anwendung");
}

#[cfg(test)]
mod tests {
    use super::{
        classifier, dfm, exporter, presets, DfmReport, DfmSummary, IpcStatus,
        StencilWorkspace, StepZone, RuleDatabase,
    };
    use crate::model::{step_zones, Component, Pad, PadShape, StepType};
    use std::path::Path;

    #[test]
    fn test_full_workflow_end_to_end() {
        // 1. Initialisiere Workspace
        let mut ws = StencilWorkspace::new();
        ws.base_thickness = 0.12;

        let pads = vec![
            Pad {
                id: 1,
                refdes: "C1".into(),
                pin: 1,
                cx: 10.0,
                cy: 10.0,
                width: 1.25,
                height: 1.40,
                shape: PadShape::Rect,
                package_name: "0805".into(),
                facing_dir: "down".into(),
                is_thermal: false,
            },
            Pad {
                id: 2,
                refdes: "C1".into(),
                pin: 2,
                cx: 12.0,
                cy: 10.0,
                width: 1.25,
                height: 1.40,
                shape: PadShape::Rect,
                package_name: "0805".into(),
                facing_dir: "up".into(),
                is_thermal: false,
            },
            Pad {
                id: 3,
                refdes: "U1".into(),
                pin: 9,
                cx: 30.0,
                cy: 30.0,
                width: 4.0,
                height: 4.0,
                shape: PadShape::Rect,
                package_name: "QFN-8".into(),
                facing_dir: "down".into(),
                is_thermal: true,
            },
        ];

        let comps = vec![
            Component {
                refdes: "C1".into(),
                package_name: "0805".into(),
                cx: 11.0,
                cy: 10.0,
                rotation: 0.0,
                side: "TOP".into(),
            },
            Component {
                refdes: "U1".into(),
                package_name: "QFN-8".into(),
                cx: 30.0,
                cy: 30.0,
                rotation: 0.0,
                side: "TOP".into(),
            },
        ];

        ws.load_dataset("top", pads, comps);

        // 2. Auto-Klassifizierung
        let stats = classifier::classify_and_apply_rules(&mut ws);
        assert!(stats.chip_count >= 1 || stats.qfn_count >= 1);

        // Prüfe non-destruktives Verhalten
        assert_eq!(ws.original_top.apertures[0].shape_type, "rectangle");
        assert_ne!(ws.working_top.apertures[0].shape_type, "rectangle");

        // 3. Stufenschablonen-Zone hinzufügen
        let zone = StepZone::new_rect("ZONE_QFN", StepType::StepDown, 0.10, 25.0, 25.0, 35.0, 35.0);
        ws.top_zones.push(zone);
        assert!(!ws.top_zones.is_empty());

        // 4. DFM-Analyse durchführen
        let ipc = dfm::analyze_ipc_parallel(&ws.working_top.apertures, ws.base_thickness, "type4");
        assert!(!ipc.is_empty());
        let bridges = dfm::check_stencil_bridges(&ws.working_top.apertures, 0.11);

        let dfm_rep = DfmReport {
            summary: DfmSummary {
                total_apertures: ws.working_top.apertures.len(),
                ok_count: ipc.iter().filter(|r| r.status == IpcStatus::Ok).count(),
                warn_count: ipc.iter().filter(|r| r.status == IpcStatus::Warn).count(),
                crit_count: ipc.iter().filter(|r| r.status == IpcStatus::Crit).count(),
                bridge_violations_count: bridges.len(),
                grain_warn_count: ipc.iter().filter(|r| r.grain_warn).count(),
                min_area_ratio: ipc.iter().map(|r| r.area_ratio).fold(f64::INFINITY, f64::min),
            },
            ipc_results: ipc,
            bridge_violations: bridges,
        };

        // 5. DFM-Report als HTML generieren
        let html = dfm::generate_html_report(&ws, &dfm_rep, "TEST_BOARD", "TOP");
        assert!(html.contains("SMT StencilCraft"));

        // 6. RS-274X Gerber-Export validieren
        let export_dir = Path::new("A:/Schablone/Export/test_out");
        let exp_res = exporter::export_stencil_gerber(
            &ws.working_top,
            &ws.top_zones,
            ws.base_thickness,
            export_dir,
            exporter::StencilSide::Top,
        );
        assert!(exp_res.is_ok());
        let res = exp_res.unwrap();
        assert!(res.zones_file.is_some());
        assert!(res.aperture_count >= 2);

        // 7. Kunden-Bibliothek Speichern / Laden testen
        let mut db = RuleDatabase::default();
        db.customer = "Automotive_Test".into();
        let save_res = presets::save_customer_library("Automotive_Test", &db);
        assert!(save_res.is_ok());
        let load_res = presets::load_customer_library("Automotive_Test");
        assert!(load_res.is_ok());
        assert_eq!(load_res.unwrap().customer, "Automotive_Test");
    }
}


