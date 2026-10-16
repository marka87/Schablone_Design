use crate::model::RuleConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDatabase {
    pub customer: String,
    pub version: String,
    pub base_thickness: f64,
    pub min_bridge: f64,
    pub packages: HashMap<String, RuleConfig>,
}

impl Default for RuleDatabase {
    fn default() -> Self {
        let mut packages = HashMap::new();
        // Standard-Vorgaben für gängige Klassen
        packages.insert(
            "0805".to_string(),
            RuleConfig {
                rule_type: "house_builder".to_string(),
                undersize_w: 0.02,
                undersize_h: 0.02,
                house_cut_pct: 10.0,
                ..Default::default()
            },
        );
        packages.insert(
            "SOIC-8".to_string(),
            RuleConfig {
                rule_type: "elongated_pitch".to_string(),
                pitch_reduction_pct: 15.0,
                corner_radius: 0.05,
                ..Default::default()
            },
        );

        Self {
            customer: "Standard_Inhouse".to_string(),
            version: "1.0".to_string(),
            base_thickness: 0.12,
            min_bridge: 0.11,
            packages,
        }
    }
}

pub fn get_libraries_dir() -> PathBuf {
    let dir = Path::new("A:/Schablone/Libraries");
    if !dir.exists() {
        let _ = fs::create_dir_all(dir);
    }
    dir.to_path_buf()
}

pub fn save_customer_library(name: &str, db: &RuleDatabase) -> Result<PathBuf, String> {
    let dir = get_libraries_dir();
    let clean_name = name.trim().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file_path = dir.join(format!("{}.json", clean_name));

    let file = File::create(&file_path)
        .map_err(|e| format!("Fehler beim Anlegen der Kundenbibliothek ({}): {}", file_path.display(), e))?;
    let writer = BufWriter::new(file);

    serde_json::to_writer_pretty(writer, db)
        .map_err(|e| format!("Fehler beim Serialisieren der Bibliothek: {}", e))?;

    Ok(file_path)
}

pub fn load_customer_library(name: &str) -> Result<RuleDatabase, String> {
    let dir = get_libraries_dir();
    let clean_name = name.trim().replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    let file_path = dir.join(format!("{}.json", clean_name));

    if !file_path.exists() {
        return Err(format!("Bibliothek '{}' nicht gefunden unter {}", name, file_path.display()));
    }

    let file = File::open(&file_path)
        .map_err(|e| format!("Fehler beim Öffnen von {}: {}", file_path.display(), e))?;
    let reader = BufReader::new(file);

    let db: RuleDatabase = serde_json::from_reader(reader)
        .map_err(|e| format!("Fehler beim Parsen der Bibliothek JSON: {}", e))?;

    Ok(db)
}

pub fn list_customer_libraries() -> Vec<String> {
    let dir = get_libraries_dir();
    let mut libs = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    libs.push(stem.to_string());
                }
            }
        }
    }

    if libs.is_empty() {
        // Standard anlegen falls leer
        let default_db = RuleDatabase::default();
        let _ = save_customer_library("Standard_Inhouse", &default_db);
        libs.push("Standard_Inhouse".to_string());
    }

    libs.sort();
    libs
}

