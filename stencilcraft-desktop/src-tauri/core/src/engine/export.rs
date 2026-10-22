//! Export für den Schablonenhersteller: Gerber je Seite (mit X2-Attributen), Stufenzonen,
//! Platinenkontur und Prüfbericht; Optionen werden im Datenverzeichnis gemerkt.

use super::gerber;
use super::types::StencilZone;
use super::StencilEngine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportSettings {
    /// Zielordner (leer = `Export/` im Datenverzeichnis)
    pub target_dir: String,
    /// Dateinamen-Muster: {job} {side} {dicke} {datum}
    pub pattern: String,
    /// "active", "top", "bot" oder "both"
    pub sides: String,
    pub zones: bool,
    pub outline: bool,
    pub report: bool,
    pub x2: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings {
            target_dir: String::new(),
            pattern: "{job}_{side}_{dicke}um".into(),
            sides: "active".into(),
            zones: true,
            outline: false,
            report: true,
            x2: true,
        }
    }
}

/// Heutiges Datum als JJJJ-MM-TT (ohne externe Abhängigkeit).
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    t.trim_matches('_').to_string()
}

/// Fügt Gerber-X2-Dateiattribute nach der Software-Kennung ein.
fn with_attributes(content: String, attrs: &[String]) -> String {
    if attrs.is_empty() {
        return content;
    }
    let marker = "%TF.GenerationSoftware";
    match content.find(marker).and_then(|p| content[p..].find('\n').map(|e| p + e + 1)) {
        Some(pos) => {
            let mut out = String::with_capacity(content.len() + 200);
            out.push_str(&content[..pos]);
            for a in attrs {
                let _ = writeln!(out, "%{a}*%");
            }
            out.push_str(&content[pos..]);
            out
        }
        None => content,
    }
}

impl StencilEngine {
    fn export_settings_path(&self) -> PathBuf {
        self.data_root.join("export_settings.json")
    }

    pub fn export_settings(&self) -> ExportSettings {
        std::fs::read_to_string(self.export_settings_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save_export_settings(&self, s: &ExportSettings) {
        if let Ok(t) = serde_json::to_string_pretty(s) {
            let _ = std::fs::write(self.export_settings_path(), t);
        }
    }

    /// Job-Name für Dateinamen: Projektname, sonst Datensatzname.
    pub fn job_name(&self) -> String {
        let raw = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .or_else(|| {
                // Name der Datensatzquelle (ohne Archiv-Endung), z. B. "flp-prg_master_v10"
                self.source_paths.first().and_then(|p| std::path::Path::new(p).file_name()).map(|n| {
                    let n = n.to_string_lossy().to_string();
                    let lower = n.to_lowercase();
                    [".tar.gz", ".tgz", ".zip", ".tar", ".gbr", ".ger"].iter().find(|e| lower.ends_with(*e)).map_or(n.clone(), |e| n[..n.len() - e.len()].to_string())
                })
            })
            .unwrap_or_else(|| self.dataset_name.split(" (").next().unwrap_or("Schablone").to_string());
        let s = sanitize(&raw);
        if s.is_empty() { "Schablone".into() } else { s }
    }

    /// Seiten mit Pastendaten in fester Reihenfolge TOP, BOT.
    pub fn loaded_sides(&self) -> Vec<String> {
        ["top", "bot"]
            .iter()
            .filter(|s| {
                if **s == self.current_side {
                    !self.apertures.is_empty()
                } else {
                    self.sides.get(**s).map_or(false, |d| !d.apertures.is_empty())
                }
            })
            .map(|s| s.to_string())
            .collect()
    }

    /// Exportiert gemäß Optionen alle gewünschten Seiten; aktive Seite und Lagen-Sichtbarkeit bleiben erhalten.
    pub fn export_all(&mut self, opts: &ExportSettings) -> Result<Value, String> {
        let dir = if opts.target_dir.trim().is_empty() { self.export_dir() } else { PathBuf::from(opts.target_dir.trim()) };
        std::fs::create_dir_all(&dir).map_err(|e| format!("Zielordner nicht anlegbar ({}): {e}", dir.display()))?;
        let sides: Vec<String> = match opts.sides.as_str() {
            "both" => self.loaded_sides(),
            "top" => vec!["top".into()],
            "bot" => vec!["bot".into()],
            _ => vec![self.current_side.clone()],
        };
        if sides.is_empty() {
            return Err("Keine Seite mit Pastendaten geladen".into());
        }
        let job = self.job_name();
        let date = today();
        let pattern = if opts.pattern.trim().is_empty() { "{job}_{side}_{dicke}um".to_string() } else { opts.pattern.clone() };
        let name_for = |side_label: &str, thick_um: f64| -> String {
            let n = pattern
                .replace("{job}", &job)
                .replace("{side}", side_label)
                .replace("{dicke}", &format!("{}", thick_um.round() as i64))
                .replace("{datum}", &date);
            // Ohne {side} im Muster würden TOP, BOT und Kontur in dieselbe Datei geschrieben
            let n = if pattern.contains("{side}") { n } else { format!("{n}_{side_label}") };
            let n = sanitize(&n);
            if n.is_empty() { format!("{job}_{side_label}") } else { n }
        };

        let original_side = self.current_side.clone();
        let visibility: Vec<(String, bool)> = self.layers.iter().map(|l| (l.id.clone(), l.visible)).collect();
        let mut files: Vec<Value> = Vec::new();
        let mut result = Ok(());

        for side in &sides {
            self.switch_side(side);
            if self.apertures.is_empty() {
                continue;
            }
            let label = side.to_uppercase();
            let base = name_for(&label, self.base_thickness * 1000.0);
            let x2_side = if side == "bot" { "Bot" } else { "Top" };

            let stencil = gerber::write_stencil(&self.apertures, &format!("Schablone {label} {:.0} um", self.base_thickness * 1000.0));
            let stencil = if opts.x2 {
                with_attributes(stencil, &[format!("TF.FileFunction,Paste,{x2_side}"), "TF.FilePolarity,Positive".into(), "TF.SameCoordinates".into()])
            } else {
                stencil
            };
            let p = dir.join(format!("{base}.gbr"));
            if let Err(e) = std::fs::write(&p, stencil) {
                result = Err(format!("{}: {e}", p.display()));
                break;
            }
            files.push(json!({"kind": "stencil", "side": side, "path": p.to_string_lossy(), "count": self.apertures.len()}));

            // Nur Stufenzonen; reine Regelzonen ändern die Blechdicke nicht
            let steps: Vec<StencilZone> = self.zones.iter().filter(|z| z.step).cloned().collect();
            if opts.zones && !steps.is_empty() {
                let z = gerber::write_zones(&steps, self.base_thickness);
                let z = if opts.x2 { with_attributes(z, &["TF.FileFunction,Other,Stufenzonen".into(), "TF.SameCoordinates".into()]) } else { z };
                let p = dir.join(format!("{base}_zonen.gbr"));
                if let Err(e) = std::fs::write(&p, z) {
                    result = Err(format!("{}: {e}", p.display()));
                    break;
                }
                files.push(json!({"kind": "zones", "side": side, "path": p.to_string_lossy(), "count": steps.len()}));
            }
        }

        if result.is_ok() && opts.outline && !self.profile_paths.is_empty() {
            let o = gerber::write_outline(&self.profile_paths);
            let o = if opts.x2 { with_attributes(o, &["TF.FileFunction,Profile,NP".into(), "TF.SameCoordinates".into()]) } else { o };
            let p = dir.join(format!("{}.gbr", name_for("Kontur", self.base_thickness * 1000.0)));
            match std::fs::write(&p, o) {
                Ok(_) => files.push(json!({"kind": "outline", "path": p.to_string_lossy()})),
                Err(e) => result = Err(format!("{}: {e}", p.display())),
            }
        }

        let mut report_file = None;
        if result.is_ok() && opts.report {
            let html = self.report_html(&sides);
            let p = dir.join(format!("{}_Pruefbericht_{date}.html", sanitize(&job)));
            match std::fs::write(&p, html) {
                Ok(_) => {
                    report_file = Some(p.to_string_lossy().to_string());
                    files.push(json!({"kind": "report", "path": p.to_string_lossy()}));
                }
                Err(e) => result = Err(format!("{}: {e}", p.display())),
            }
        }

        // Ursprünglichen Zustand wiederherstellen
        self.switch_side(&original_side);
        for (id, vis) in visibility {
            if let Some(l) = self.layer_mut(&id) {
                l.visible = vis;
            }
        }
        result?;

        let stencil_files: Vec<String> = files.iter().filter(|f| f["kind"] == "stencil").filter_map(|f| f["path"].as_str().map(String::from)).collect();
        Ok(json!({
            "status": "ok",
            "export_dir": dir.to_string_lossy(),
            "files": files,
            "report_file": report_file,
            "stencil_file": stencil_files.first(),
            "message": format!("{} Datei(en) exportiert nach {}", files.len(), dir.display()),
        }))
    }
}
