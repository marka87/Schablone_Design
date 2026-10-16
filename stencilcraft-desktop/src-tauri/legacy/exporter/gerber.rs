use crate::model::{Layer, StepZone};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StencilSide {
    Top,
    Bot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub side: StencilSide,
    pub stencil_file: String,
    pub zones_file: Option<String>,
    pub aperture_count: usize,
}

/// Konvertiert eine Millimeter-Koordinate in RS-274X Format 4.6 (1 mm = 1.000.000 Einheiten).
#[inline]
pub fn fmt_coord_46(val_mm: f64) -> String {
    let units = (val_mm * 1_000_000.0).round() as i64;
    format!("{}", units)
}

/// Generiert den normgerechten RS-274X Gerber-Code für eine Schablonen-Arbeitslage.
pub fn generate_rs274x_content(layer: &Layer) -> String {
    let mut out = String::with_capacity(layer.apertures.len() * 128);

    // 1. Gerber RS-274X Header
    out.push_str("G04 SMT StencilCraft v2.6 - Laser Stencil Layer*\n");
    out.push_str(&format!("G04 Layer: {} ({})*\n", layer.name, layer.side));
    out.push_str("%FSLAX46Y46*%\n"); // Format 4.6 Absolute leading zeros omitted
    out.push_str("%MOMM*%\n");       // Metric Millimeters
    out.push_str("%LPD*%\n");        // Layer Polarity Dark
    out.push_str("G90*\n");          // Absolute coordinates
    out.push_str("G71*\n");          // Metric mode

    // 2. Apertur-Dictionary aufbauen für Standard-Rechtecke & Kreise
    let mut dcode_counter = 10;
    let mut rect_dict: HashMap<(i64, i64), usize> = HashMap::new();
    let mut circle_dict: HashMap<i64, usize> = HashMap::new();

    for ap in &layer.apertures {
        if ap.points.is_empty() || ap.shape_type == "rectangle" || ap.shape_type == "simple_rect" {
            let key_w = (ap.width * 1000.0).round() as i64;
            let key_h = (ap.height * 1000.0).round() as i64;
            rect_dict.entry((key_w, key_h)).or_insert_with(|| {
                let code = dcode_counter;
                dcode_counter += 1;
                code
            });
        } else if ap.shape_type == "circle" {
            let key_d = (ap.width * 1000.0).round() as i64;
            circle_dict.entry(key_d).or_insert_with(|| {
                let code = dcode_counter;
                dcode_counter += 1;
                code
            });
        }
    }

    // 3. Apertur-Definitionen (%ADD...*%) in Header schreiben
    for (&(w_key, h_key), &code) in &rect_dict {
        let w = w_key as f64 / 1000.0;
        let h = h_key as f64 / 1000.0;
        out.push_str(&format!("%ADD{}R,{:.4}X{:.4}*%\n", code, w, h));
    }
    for (&d_key, &code) in &circle_dict {
        let d = d_key as f64 / 1000.0;
        out.push_str(&format!("%ADD{}C,{:.4}*%\n", code, d));
    }

    // 4. Aperturen platzieren
    let mut current_dcode: Option<usize> = None;

    for ap in &layer.apertures {
        // Fall A: Komplexe Polygone (House, Inverted House, 45°-Slices, Chamfers) -> G36/G37 Polygone
        if ap.points.len() >= 3 && ap.shape_type != "rectangle" && ap.shape_type != "simple_rect" && ap.shape_type != "circle" {
            out.push_str("G36*\n"); // Start Polygon Fill
            for (idx, p) in ap.points.iter().enumerate() {
                let op = if idx == 0 { "02" } else { "01" };
                out.push_str(&format!("X{}Y{}D{}*\n", fmt_coord_46(p.x), fmt_coord_46(p.y), op));
            }
            // Polygon schließen
            if let Some(first) = ap.points.first() {
                out.push_str(&format!("X{}Y{}D01*\n", fmt_coord_46(first.x), fmt_coord_46(first.y)));
            }
            out.push_str("G37*\n"); // End Polygon Fill
        }
        // Fall B: Standard Kreis-Flashes
        else if ap.shape_type == "circle" {
            let key_d = (ap.width * 1000.0).round() as i64;
            if let Some(&code) = circle_dict.get(&key_d) {
                if current_dcode != Some(code) {
                    out.push_str(&format!("D{}*\n", code));
                    current_dcode = Some(code);
                }
                out.push_str(&format!("X{}Y{}D03*\n", fmt_coord_46(ap.cx), fmt_coord_46(ap.cy)));
            }
        }
        // Fall C: Standard Rechteck-Flashes
        else {
            let key_w = (ap.width * 1000.0).round() as i64;
            let key_h = (ap.height * 1000.0).round() as i64;
            if let Some(&code) = rect_dict.get(&(key_w, key_h)) {
                if current_dcode != Some(code) {
                    out.push_str(&format!("D{}*\n", code));
                    current_dcode = Some(code);
                }
                out.push_str(&format!("X{}Y{}D03*\n", fmt_coord_46(ap.cx), fmt_coord_46(ap.cy)));
            }
        }
    }

    out.push_str("M02*\n"); // End of File
    out
}

/// Schreibt die Schablone der gewünschten Seite in eine laserfertige Gerber-Datei.
/// Generiert den normgerechten RS-274X Gerber-Code für Stufenschablonen-Zonen (Fräs-/Ätzkonturen).
pub fn generate_step_zones_gerber(zones: &[StepZone], base_thickness: f64) -> String {
    let mut out = String::new();
    out.push_str("G04 SMT StencilCraft v2.6 - Step Stencil Zones (Milling/Etch Boundaries)*\n");
    out.push_str(&format!("G04 Base Thickness: {:.4} mm*\n", base_thickness));
    out.push_str("%FSLAX46Y46*%\n");
    out.push_str("%MOMM*%\n");
    out.push_str("%LPD*%\n");
    out.push_str("G90*\n");
    out.push_str("G71*\n");
    out.push_str("%ADD10C,0.2000*%\n"); // 0.20 mm Fräslinie
    out.push_str("D10*\n");

    for zone in zones {
        out.push_str(&format!("G04 Zone: {} Type: {:?} Thick: {:.4} mm*\n", zone.id, zone.zone_type, zone.thickness));
        if zone.points.len() >= 3 {
            for (idx, p) in zone.points.iter().enumerate() {
                let op = if idx == 0 { "02" } else { "01" };
                out.push_str(&format!("X{}Y{}D{}*\n", fmt_coord_46(p.x), fmt_coord_46(p.y), op));
            }
            if let Some(first) = zone.points.first() {
                out.push_str(&format!("X{}Y{}D01*\n", fmt_coord_46(first.x), fmt_coord_46(first.y)));
            }
        }
    }

    out.push_str("M02*\n");
    out
}

/// Schreibt die Schablone der gewünschten Seite (inklusive optionaler Stufenzonen) in laserfertige Gerber-Dateien.
pub fn export_stencil_gerber(
    layer: &Layer,
    zones: &[StepZone],
    base_thickness: f64,
    target_dir: &Path,
    side: StencilSide,
) -> Result<ExportResult, String> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Konnte Zielverzeichnis nicht erstellen: {}", e))?;

    let filename = match side {
        StencilSide::Top => "stencil_top.ger",
        StencilSide::Bot => "stencil_bot.ger",
    };

    let filepath = target_dir.join(filename);
    let gerber_code = generate_rs274x_content(layer);

    let file = File::create(&filepath)
        .map_err(|e| format!("Konnte Gerber-Datei nicht anlegen ({}): {}", filepath.display(), e))?;
    let mut writer = BufWriter::new(file);

    writer
        .write_all(gerber_code.as_bytes())
        .map_err(|e| format!("Fehler beim Schreiben der Gerber-Datei: {}", e))?;
    writer
        .flush()
        .map_err(|e| format!("Fehler beim Schließen der Datei: {}", e))?;

    // Optionale Stufenzonen-Datei erzeugen
    let zones_file = if !zones.is_empty() {
        let z_name = match side {
            StencilSide::Top => "stencil_top_zones.ger",
            StencilSide::Bot => "stencil_bot_zones.ger",
        };
        let z_path = target_dir.join(z_name);
        let z_code = generate_step_zones_gerber(zones, base_thickness);
        let z_file = File::create(&z_path)
            .map_err(|e| format!("Konnte Zonen-Gerber nicht anlegen: {}", e))?;
        let mut z_writer = BufWriter::new(z_file);
        z_writer.write_all(z_code.as_bytes()).map_err(|e| format!("Fehler beim Schreiben der Zonen: {}", e))?;
        z_writer.flush().map_err(|e| format!("Fehler beim Flush der Zonen: {}", e))?;
        Some(z_path.to_string_lossy().to_string())
    } else {
        None
    };

    Ok(ExportResult {
        side,
        stencil_file: filepath.to_string_lossy().to_string(),
        zones_file,
        aperture_count: layer.apertures.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Aperture, Point};

    #[test]
    fn test_gerber_export_content() {
        let mut layer = Layer::new("work_top", "Schablone TOP", crate::model::LayerType::StencilWorking, "TOP", false, "#fff");
        
        // 1 Standard-Rechteck
        layer.apertures.push(Aperture {
            id: 1,
            pad_id: Some(1),
            package_name: "0805".into(),
            pin: 1,
            cx: 10.0,
            cy: 15.0,
            width: 1.2,
            height: 1.3,
            points: vec![],
            area: 1.56,
            shape_type: "rectangle".into(),
            is_modified: false,
            manual_override: false,
            ..Default::default()
        });

        // 1 Polygon (HouseBuilder)
        layer.apertures.push(Aperture {
            id: 2,
            pad_id: Some(2),
            package_name: "0805".into(),
            pin: 2,
            cx: 12.0,
            cy: 15.0,
            width: 1.2,
            height: 1.3,
            points: vec![
                Point::new(11.4, 14.35),
                Point::new(12.6, 14.35),
                Point::new(12.6, 15.2),
                Point::new(12.0, 15.65),
                Point::new(11.4, 15.2),
            ],
            area: 1.40,
            shape_type: "house_builder".into(),
            is_modified: true,
            manual_override: false,
            ..Default::default()
        });

        let content = generate_rs274x_content(&layer);
        assert!(content.contains("%FSLAX46Y46*%"));
        assert!(content.contains("%MOMM*%"));
        assert!(content.contains("%ADD10R,1.2000X1.3000*%"));
        assert!(content.contains("G36*"));
        assert!(content.contains("G37*"));
        assert!(content.contains("M02*"));
    }
}
