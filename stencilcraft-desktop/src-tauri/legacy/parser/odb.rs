use crate::model::{Aperture, Component, Pad, PadShape, StencilWorkspace};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use tar::Archive as TarArchive;
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerMeta {
    pub id: String,
    pub name: String,
    pub layer_type: String,
    pub side: String,
    pub count: usize,
    pub color: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub job_name: String,
    pub step_name: String,
    pub layers: Vec<LayerMeta>,
    pub components_count: usize,
    pub apertures_top_count: usize,
    pub apertures_bot_count: usize,
    pub bounds: [f64; 4], // [min_x, min_y, max_x, max_y]
}

#[derive(Debug, Clone)]
struct SymbolDef {
    shape: PadShape,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct EdaPin {
    pin_name: String,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone)]
struct EdaPackage {
    name: String,
    pitch: f64,
    width: f64,
    height: f64,
    pins: Vec<EdaPin>,
}

/// Liest relevante ODB++ Textdateien aus ZIP, TGZ oder einem lokalen Verzeichnis in den Arbeitsspeicher.
fn load_odb_files<P: AsRef<Path>>(path: P) -> Result<HashMap<String, String>, String> {
    let path = path.as_ref();
    let mut files = HashMap::new();

    if path.is_dir() {
        // 1. Unpacked Directory
        for entry in walkdir(path)? {
            let p = entry.as_path();
            if p.is_file() {
                let norm = normalize_path(p.strip_prefix(path).unwrap_or(p));
                if is_relevant_odb_path(&norm) {
                    if let Ok(content) = std::fs::read_to_string(p) {
                        files.insert(norm, content);
                    }
                }
            }
        }
    } else {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let file = File::open(path).map_err(|e| format!("Fehler beim Öffnen von {}: {}", path.display(), e))?;
        let buf = BufReader::new(file);

        if ext == "zip" {
            // 2. ZIP Archiv
            let mut archive = ZipArchive::new(buf).map_err(|e| format!("Ungültiges ZIP Archiv: {}", e))?;
            for i in 0..archive.len() {
                if let Ok(mut zip_file) = archive.by_index(i) {
                    let norm = normalize_path(Path::new(zip_file.name()));
                    if is_relevant_odb_path(&norm) {
                        let mut content = String::new();
                        if zip_file.read_to_string(&mut content).is_ok() {
                            files.insert(norm, content);
                        }
                    }
                }
            }
        } else if ext == "tgz" || ext == "gz" || path.to_string_lossy().ends_with(".tar.gz") {
            // 3. TAR.GZ / TGZ Archiv
            let gz = GzDecoder::new(buf);
            let mut tar = TarArchive::new(gz);
            if let Ok(entries) = tar.entries() {
                for entry in entries.flatten() {
                    if let Ok(p) = entry.path() {
                        let norm = normalize_path(&p);
                        if is_relevant_odb_path(&norm) {
                            let mut content = String::new();
                            let mut reader = BufReader::new(entry);
                            if reader.read_to_string(&mut content).is_ok() {
                                files.insert(norm, content);
                            }
                        }
                    }
                }
            }
        } else {
            return Err(format!("Nicht unterstütztes Format: {}", path.display()));
        }
    }

    if files.is_empty() {
        return Err("Keine gültigen ODB++ Datensätze im angegebenen Pfad gefunden.".into());
    }

    Ok(files)
}

fn walkdir(dir: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    let mut result = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                if let Ok(sub) = walkdir(&p) {
                    result.extend(sub);
                }
            } else {
                result.push(p);
            }
        }
    }
    Ok(result)
}

fn normalize_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_start_matches("./").to_lowercase()
}

fn is_relevant_odb_path(p: &str) -> bool {
    p.contains("matrix/matrix") || p.ends_with("/matrix") ||
    p.contains("/eda/data") ||
    p.contains("/features") ||
    p.contains("/components")
}

/// Parst ODB++ Symbol-Definitionen (z. B. rect60x20, rect30.5x7.8xr3.1, r50, s40, oval60x20).
fn parse_symbol_string(sdef: &str, is_inch: bool) -> SymbolDef {
    // In ODB++ sind Symbole bei UNITS=INCH in mils (1 mil = 0.0254 mm) angegeben.
    // Bei UNITS=MM sind sie in mm (oder microns falls > 50).
    let unit_m = if is_inch { 0.0254 } else { 1.0 };
    let s = sdef.to_lowercase();

    // 1. rect<w>x<h> oder rect<w>x<h>xr<r>
    if s.starts_with("rect") {
        let rest = &s[4..];
        let parts: Vec<&str> = rest.split('x').collect();
        if parts.len() >= 2 {
            let mut w = parts[0].parse::<f64>().unwrap_or(30.0) * unit_m;
            let mut h = parts[1].parse::<f64>().unwrap_or(30.0) * unit_m;
            if !is_inch && w > 50.0 { w /= 1000.0; }
            if !is_inch && h > 50.0 { h /= 1000.0; }
            return SymbolDef { shape: PadShape::Rect, width: w.max(0.02), height: h.max(0.02) };
        }
    }
    // 2. oval<w>x<h>
    else if s.starts_with("oval") {
        let rest = &s[4..];
        let parts: Vec<&str> = rest.split('x').collect();
        if parts.len() >= 2 {
            let mut w = parts[0].parse::<f64>().unwrap_or(30.0) * unit_m;
            let mut h = parts[1].parse::<f64>().unwrap_or(30.0) * unit_m;
            if !is_inch && w > 50.0 { w /= 1000.0; }
            if !is_inch && h > 50.0 { h /= 1000.0; }
            return SymbolDef { shape: PadShape::Round, width: w.max(0.02), height: h.max(0.02) };
        }
    }
    // 3. s<side> (Quadrat)
    else if s.starts_with('s') {
        let rest = &s[1..];
        let mut side = rest.parse::<f64>().unwrap_or(30.0) * unit_m;
        if !is_inch && side > 50.0 { side /= 1000.0; }
        return SymbolDef { shape: PadShape::Rect, width: side.max(0.02), height: side.max(0.02) };
    }
    // 4. round<dia> oder r<dia> oder c<dia> (Kreis)
    else if s.starts_with("round") {
        let rest = &s[5..];
        let mut dia = rest.parse::<f64>().unwrap_or(30.0) * unit_m;
        if !is_inch && dia > 50.0 { dia /= 1000.0; }
        return SymbolDef { shape: PadShape::Round, width: dia.max(0.02), height: dia.max(0.02) };
    }
    else if s.starts_with('r') {
        let rest = &s[1..];
        let mut dia = rest.parse::<f64>().unwrap_or(30.0) * unit_m;
        if !is_inch && dia > 50.0 { dia /= 1000.0; }
        return SymbolDef { shape: PadShape::Round, width: dia.max(0.02), height: dia.max(0.02) };
    }
    else if s.starts_with('c') {
        let rest = &s[1..];
        let mut dia = rest.parse::<f64>().unwrap_or(30.0) * unit_m;
        if !is_inch && dia > 50.0 { dia /= 1000.0; }
        return SymbolDef { shape: PadShape::Round, width: dia.max(0.02), height: dia.max(0.02) };
    }

    SymbolDef { shape: PadShape::Rect, width: 0.8, height: 0.8 }
}

/// Parst die eda/data Datei zur Rekonstruktion echter Pad-Geometrien (OB, OS, OC, RC, CR).
fn parse_eda_data(content: &str) -> HashMap<String, EdaPackage> {
    let mut packages = HashMap::new();
    let mut current_pkg: Option<EdaPackage> = None;
    let mut current_pin: Option<EdaPin> = None;
    let mut scale = 25.4; // Default INCH in ODB++

    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }

        if l.starts_with("UNITS=") {
            scale = if l.contains("MM") { 1.0 } else { 25.4 };
            continue;
        }

        if l.starts_with("PKG ") {
            if let Some(mut pkg) = current_pkg.take() {
                if let Some(pin) = current_pin.take() {
                    pkg.pins.push(pin);
                }
                packages.insert(pkg.name.clone(), pkg);
            }
            let parts: Vec<&str> = l.split_whitespace().collect();
            let name = parts.get(1).unwrap_or(&"UNKNOWN").to_string();
            let pitch = parts.get(2).and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0) * scale;
            current_pkg = Some(EdaPackage {
                name,
                pitch,
                width: 2.0,
                height: 2.0,
                pins: Vec::new(),
            });
        } else if l.starts_with("PIN ") {
            if let Some(ref mut pkg) = current_pkg {
                if let Some(pin) = current_pin.take() {
                    pkg.pins.push(pin);
                }
                let parts: Vec<&str> = l.split_whitespace().collect();
                let pin_name = parts.get(1).unwrap_or(&"1").to_string();
                current_pin = Some(EdaPin {
                    pin_name,
                    width: 0.8,
                    height: 0.8,
                });
            }
        } else if l.starts_with("RC ") {
            // Rechteck-Pad Geometrie: RC <x> <y> <w> <h>
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 5 {
                if let (Ok(rw), Ok(rh)) = (parts[3].parse::<f64>(), parts[4].parse::<f64>()) {
                    if let Some(ref mut pin) = current_pin {
                        pin.width = (rw * scale).abs();
                        pin.height = (rh * scale).abs();
                    }
                }
            }
        } else if l.starts_with("CR ") {
            // Kreis-Pad Geometrie: CR <x> <y> <r>
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 4 {
                if let Ok(cr) = parts[3].parse::<f64>() {
                    if let Some(ref mut pin) = current_pin {
                        let dia = (2.0 * cr * scale).abs();
                        pin.width = dia;
                        pin.height = dia;
                    }
                }
            }
        }
    }

    if let Some(mut pkg) = current_pkg {
        if let Some(pin) = current_pin {
            pkg.pins.push(pin);
        }
        packages.insert(pkg.name.clone(), pkg);
    }

    packages
}

/// Parst die components Datei einer Bestückungsseite.
fn parse_components_file(content: &str) -> Vec<Component> {
    let mut comps = Vec::new();
    let mut scale = 25.4;

    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }

        if l.starts_with("UNITS=") {
            scale = if l.contains("MM") { 1.0 } else { 25.4 };
            continue;
        }

        if l.starts_with("CMP ") {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 6 {
                let cx = parts[2].parse::<f64>().unwrap_or(0.0) * scale;
                let cy = parts[3].parse::<f64>().unwrap_or(0.0) * scale;
                let rot = parts[4].parse::<f64>().unwrap_or(0.0);
                let refdes = parts.get(6).unwrap_or(&parts[5]).split(';').next().unwrap_or("U1").to_string();

                comps.push(Component {
                    refdes,
                    package_name: "SMD".to_string(),
                    cx,
                    cy,
                    rotation: rot,
                    side: "TOP".to_string(),
                });
            }
        }
    }
    comps
}

/// Parst eine ODB++ Features Datei für Kupfer oder Pastenlagen.
fn parse_features_file(content: &str, _layer_side: &str, comps: &[Component]) -> (Vec<Pad>, Vec<Aperture>) {
    let mut pads = Vec::new();
    let mut apertures = Vec::new();
    let mut symbols: HashMap<String, SymbolDef> = HashMap::new();
    let mut scale: f64 = 25.4;

    for line in content.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('@') || l.starts_with('&') {
            continue;
        }

        if l.starts_with("UNITS=") {
            scale = if l.contains("MM") { 1.0 } else { 25.4 };
            continue;
        }

        if l.starts_with('$') {
            let parts: Vec<&str> = l[1..].split_whitespace().collect();
            if parts.len() >= 2 {
                let is_inch = scale > 2.0;
                symbols.insert(parts[0].to_string(), parse_symbol_string(parts[1], is_inch));
            }
            continue;
        }

        if l.starts_with("P ") {
            let parts: Vec<&str> = l.split_whitespace().collect();
            if parts.len() >= 4 {
                let cx = parts[1].parse::<f64>().unwrap_or(0.0) * scale;
                let cy = parts[2].parse::<f64>().unwrap_or(0.0) * scale;
                let sid = parts[3];
                let rot = parts.get(5).and_then(|r| r.parse::<f64>().ok()).unwrap_or(0.0);

                let default_sym = SymbolDef { shape: PadShape::Rect, width: 0.8, height: 0.8 };
                let sym = symbols.get(sid).unwrap_or(&default_sym);

                let is_90 = (rot - 90.0).abs() < 1.0 || (rot - 270.0).abs() < 1.0;
                let (pad_w, pad_h) = if is_90 {
                    (sym.height, sym.width)
                } else {
                    (sym.width, sym.height)
                };

                let id = pads.len();

                // Nahegelegenes Bauteil suchen für RefDes & Package
                let mut best_comp = None;
                let mut min_d = 5.0; // mm
                for c in comps {
                    let d = ((c.cx - cx).powi(2) + (c.cy - cy).powi(2)).sqrt();
                    if d < min_d {
                        min_d = d;
                        best_comp = Some(c);
                    }
                }

                let (refdes, pkg_name) = match best_comp {
                    Some(c) => (c.refdes.clone(), c.package_name.clone()),
                    None => (format!("PAD_{}", id + 1), format!("{:.2}x{:.2}", pad_w, pad_h)),
                };

                let pad = Pad {
                    id,
                    refdes,
                    pin: 1,
                    cx,
                    cy,
                    width: pad_w,
                    height: pad_h,
                    shape: sym.shape.clone(),
                    package_name: pkg_name,
                    facing_dir: "down".to_string(),
                    is_thermal: pad_w > 2.5 && pad_h > 2.5,
                };

                let mut ap = Aperture::from_pad(&pad);
                ap.rotation = rot;
                pads.push(pad);
                apertures.push(ap);
            }
        }
    }

    (pads, apertures)
}

/// Parst ein gesamtes ODB++ Archiv (.zip, .tgz, Ordner) und initialisiert den non-destruktiven StencilWorkspace.
pub fn parse_odb_archive<P: AsRef<Path>>(
    archive_path: P,
    workspace: &mut StencilWorkspace,
) -> Result<WorkspaceSummary, String> {
    let files = load_odb_files(&archive_path)?;
    if files.is_empty() {
        return Err(format!(
            "Beschädigter oder unvollständiger ODB++ Datensatz in '{}': Keine lesbaren ODB++ Dateien gefunden (matrix oder steps fehlen).",
            archive_path.as_ref().display()
        ));
    }

    // 1. Step-Name ermitteln (z. B. steps/stp/eda/data -> "stp")
    let mut step_name = "stp".to_string();
    let mut step_found = false;
    for key in files.keys() {
        if key.contains("steps/") && key.contains("/eda/data") {
            let parts: Vec<&str> = key.split('/').collect();
            if let Some(pos) = parts.iter().position(|&x| x == "steps") {
                if let Some(step) = parts.get(pos + 1) {
                    step_name = step.to_string();
                    step_found = true;
                    break;
                }
            }
        }
    }

    if !step_found {
        for key in files.keys() {
            if key.contains("steps/") && key.contains("/layers/") {
                let parts: Vec<&str> = key.split('/').collect();
                if let Some(pos) = parts.iter().position(|&x| x == "steps") {
                    if let Some(step) = parts.get(pos + 1) {
                        step_name = step.to_string();
                        break;
                    }
                }
            }
        }
    }

    // 2. EDA Packages einlesen
    let eda_key = format!("steps/{}/eda/data", step_name);
    let _eda_pkgs = if let Some(content) = files.get(&eda_key) {
        parse_eda_data(content)
    } else {
        HashMap::new()
    };

    // 3. Komponenten einlesen
    let mut all_comps = Vec::new();
    let comp_top_key = format!("steps/{}/layers/comp_+_top/components", step_name);
    if let Some(c_top) = files.get(&comp_top_key) {
        all_comps.extend(parse_components_file(c_top));
    }
    let comp_bot_key = format!("steps/{}/layers/comp_+_bot/components", step_name);
    if let Some(c_bot) = files.get(&comp_bot_key) {
        all_comps.extend(parse_components_file(c_bot));
    }

    // 4. Pasten- und Kupfer-Lagen finden und parsen
    let mut top_pads = Vec::new();
    let mut top_apertures = Vec::new();
    let mut bot_pads = Vec::new();
    let mut bot_apertures = Vec::new();
    let mut layer_metas = Vec::new();

    // Suche Features nach typischen Paste-Namen
    for (path, content) in &files {
        if path.contains(&format!("steps/{}/layers/", step_name)) && path.ends_with("/features") {
            let parts: Vec<&str> = path.split('/').collect();
            let layer_name = parts.get(parts.len() - 2).unwrap_or(&"unknown");
            let lname = layer_name.to_lowercase();

            let is_top_paste = lname == "spt" || lname.contains("top_paste") || (lname.contains("paste") && lname.contains("top"));
            let is_bot_paste = lname == "spb" || lname.contains("bot_paste") || (lname.contains("paste") && (lname.contains("bot") || lname.contains("bottom")));

            if is_top_paste {
                let (p, a) = parse_features_file(content, "TOP", &all_comps);
                layer_metas.push(LayerMeta {
                    id: format!("layer_{}", lname),
                    name: format!("TOP_PASTE ({})", layer_name),
                    layer_type: "paste".into(),
                    side: "TOP".into(),
                    count: a.len(),
                    color: "#e879f9".into(),
                    visible: true,
                });
                top_pads = p;
                top_apertures = a;
            } else if is_bot_paste {
                let (p, a) = parse_features_file(content, "BOT", &all_comps);
                layer_metas.push(LayerMeta {
                    id: format!("layer_{}", lname),
                    name: format!("BOT_PASTE ({})", layer_name),
                    layer_type: "paste".into(),
                    side: "BOT".into(),
                    count: a.len(),
                    color: "#c084fc".into(),
                    visible: true,
                });
                bot_pads = p;
                bot_apertures = a;
            } else if lname.contains("top") || lname.contains("bot") {
                // Signal / Copper / Mask Metadaten
                let ltype = if lname.contains("sm") || lname.contains("mask") { "mask" } else { "copper" };
                let side = if lname.contains("bot") { "BOT" } else { "TOP" };
                layer_metas.push(LayerMeta {
                    id: format!("layer_{}", lname),
                    name: layer_name.to_string(),
                    layer_type: ltype.into(),
                    side: side.into(),
                    count: content.lines().filter(|l| l.starts_with("P ") || l.starts_with("L ")).count(),
                    color: if ltype == "mask" { "#06b6d4".into() } else { "#10b981".into() },
                    visible: false,
                });
            }
        }
    }

    // 5. In StencilWorkspace ablegen (strikte Trennung: Original geschützt, Working modifizierbar)
    let ap_top_cnt = top_apertures.len();
    let ap_bot_cnt = bot_apertures.len();

    workspace.original_top.pads = top_pads.clone();
    workspace.original_top.apertures = top_apertures.clone();
    workspace.working_top.pads = top_pads;
    workspace.working_top.apertures = top_apertures;

    workspace.original_bot.pads = bot_pads.clone();
    workspace.original_bot.apertures = bot_apertures.clone();
    workspace.working_bot.pads = bot_pads;
    workspace.working_bot.apertures = bot_apertures;

    workspace.components = all_comps;

    // 6. Bounding Box berechnen
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for ap in workspace.working_top.apertures.iter().chain(workspace.working_bot.apertures.iter()) {
        if ap.cx - ap.width / 2.0 < min_x { min_x = ap.cx - ap.width / 2.0; }
        if ap.cx + ap.width / 2.0 > max_x { max_x = ap.cx + ap.width / 2.0; }
        if ap.cy - ap.height / 2.0 < min_y { min_y = ap.cy - ap.height / 2.0; }
        if ap.cy + ap.height / 2.0 > max_y { max_y = ap.cy + ap.height / 2.0; }
    }

    let bounds = if min_x.is_infinite() {
        [0.0, 0.0, 100.0, 100.0]
    } else {
        [min_x - 5.0, min_y - 5.0, max_x + 5.0, max_y + 5.0]
    };

    let job_name = archive_path
        .as_ref()
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("odb_job")
        .to_string();

    Ok(WorkspaceSummary {
        job_name,
        step_name,
        layers: layer_metas,
        components_count: workspace.components.len(),
        apertures_top_count: ap_top_cnt,
        apertures_bot_count: ap_bot_cnt,
        bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_symbol_string() {
        let sym_r = parse_symbol_string("rect30.5118x7.874xr3.1496", true);
        assert_eq!(sym_r.shape, PadShape::Rect);
        assert!((sym_r.width - 0.775).abs() < 1e-3);
        assert!((sym_r.height - 0.200).abs() < 1e-3);

        let sym_c = parse_symbol_string("r19.685", true);
        assert_eq!(sym_c.shape, PadShape::Round);
        assert!((sym_c.width - 0.500).abs() < 1e-3);
    }

    #[test]
    fn test_parse_eda_pin_contours() {
        let eda_mock = "
UNITS=MM
PKG 0805 1.702 0 0 2.0 1.25
PIN 1 SMD -0.851 0
RC -0.851 0 1.20 1.30
PIN 2 SMD 0.851 0
RC 0.851 0 1.20 1.30
";
        let pkgs = parse_eda_data(eda_mock);
        let pkg = pkgs.get("0805").expect("0805 package not found");
        assert_eq!(pkg.pins.len(), 2);
        assert!((pkg.pins[0].width - 1.20).abs() < 1e-4);
        assert!((pkg.pins[0].height - 1.30).abs() < 1e-4);
    }

    #[test]
    fn test_parse_features() {
        let feat_mock = "
UNITS=MM
$1 r1200x1300
P 10.0 15.0 1 0 0
";
        let (pads, apertures) = parse_features_file(feat_mock, "TOP", &[]);
        assert_eq!(pads.len(), 1);
        assert_eq!(apertures.len(), 1);
        assert!((pads[0].cx - 10.0).abs() < 1e-4);
        assert!((pads[0].cy - 15.0).abs() < 1e-4);
        assert!((pads[0].width - 1.2).abs() < 1e-4);
    }
}

