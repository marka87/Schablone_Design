//! SMT StencilCraft Desktop – Tauri-Backend.
//!
//! Die Web-Oberfläche (identisch zur Flask-Version) spricht unverändert `/api/...` an.
//! `ui/bridge.js` leitet diese Aufrufe über `api_request` an den nativen Router (`api.rs`)
//! und die Rust-Engine (`engine/`) weiter.

use stencil_core::api::{self, Dialogs};
use stencil_core::engine::{self, StencilEngine};
use serde::Serialize;
use serde_json::Value;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};

pub struct AppState {
    pub engine: Mutex<StencilEngine>,
    /// Aktuelle UI-Skalierung (Seitenzoom) aller Fenster
    pub ui_zoom: Mutex<f64>,
}

/// Fenster-Label einer Zusatzansicht ("package" → eigenes Stencil-Generator-Fenster)
fn view_label(view: &str) -> Option<&'static str> {
    match view {
        "package" | "generator" => Some("generator"),
        _ => None,
    }
}

#[derive(Serialize)]
pub struct ApiResponse {
    status: u16,
    body: Value,
}

/// Native Windows-Dialoge (Common Item Dialog) über `rfd`.
struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn pick_files(&self) -> Option<Vec<PathBuf>> {
        rfd::FileDialog::new()
            .set_title("ODB++ / Gerber Datensatz öffnen")
            .add_filter("ODB++ / Gerber", &["zip", "tgz", "gz", "tar", "ger", "gbr", "pho", "art", "gtp", "gbp", "gts", "gbs", "gtl", "gbl", "gko"])
            .add_filter("Alle Dateien", &["*"])
            .pick_files()
    }
    fn pick_folder(&self) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title("ODB++ / Gerber Ordner öffnen").pick_folder()
    }
    fn pick_project(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("StencilCraft-Projekt öffnen")
            .add_filter("StencilCraft-Projekt", &["stencilproj"])
            .pick_file()
    }
    fn pick_save_project(&self, default_name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("StencilCraft-Projekt speichern")
            .add_filter("StencilCraft-Projekt", &["stencilproj"])
            .set_file_name(format!("{default_name}.stencilproj"))
            .save_file()
    }
    fn pick_rule_tree(&self) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("PCB-Investigator-Kundenbibliothek importieren")
            .add_filter("PCB-I Rule Tree", &["xml"])
            .pick_file()
    }
    fn pick_save_rule_tree(&self, default_name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title("Als PCB-Investigator-Kundenbibliothek exportieren")
            .add_filter("PCB-I Rule Tree", &["xml"])
            .set_file_name(format!("{default_name}.xml"))
            .save_file()
    }
}

fn upload_root() -> PathBuf {
    std::env::temp_dir().join("stencilcraft_upload")
}

/// Führt einen API-Aufruf der Web-Oberfläche aus (Methode, URL inkl. Query, JSON-Body).
/// Läuft in einem Hintergrund-Thread, damit Import/DFM die Oberfläche nicht blockieren.
#[tauri::command]
async fn api_request(app: tauri::AppHandle, window: tauri::WebviewWindow, method: String, url: String, body: Option<Value>) -> Result<ApiResponse, String> {
    let source = window.label().to_string();
    let response = tauri::async_runtime::spawn_blocking(move || {
        let state: State<'_, AppState> = app.state();
        let body = body.unwrap_or(Value::Null);
        let result = catch_unwind(AssertUnwindSafe(|| api::dispatch(&state.engine, &method.to_uppercase(), &url, &body, &NativeDialogs)));
        let (status, body) = result.unwrap_or_else(|p| {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "Unbekannter Fehler".into());
            (500, serde_json::json!({"status": "error", "message": format!("Interner Fehler: {msg}")}))
        });
        // Andere Fenster über Änderungen informieren (sie laden ihre Ansicht nach)
        let method_uc = method.to_uppercase();
        if method_uc != "GET" && status < 400 {
            let _ = app.emit("engine-changed", serde_json::json!({"path": url, "method": method_uc, "source": source}));
        }
        ApiResponse { status, body }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(response)
}

/// Öffnet (oder fokussiert) eine Zusatzansicht in eigenem Fenster, z. B. den Stencil Generator.
/// Async, weil das Erzeugen von Fenstern in synchronen Commands unter Windows blockieren kann.
#[tauri::command]
async fn open_view_window(app: tauri::AppHandle, view: String) -> Result<(), String> {
    let label = view_label(&view).ok_or("Unbekannte Ansicht")?;
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let w = WebviewWindowBuilder::new(&app, label, WebviewUrl::App("index.html?view=package".into()))
        .title("Stencil Generator – SMT StencilCraft")
        .inner_size(1500.0, 950.0)
        .min_inner_size(900.0, 600.0)
        .resizable(true)
        .disable_drag_drop_handler()
        .zoom_hotkeys_enabled(true)
        .build()
        .map_err(|e| e.to_string())?;
    let zoom = *app.state::<AppState>().ui_zoom.lock().unwrap_or_else(|p| p.into_inner());
    let _ = w.set_zoom(zoom);
    Ok(())
}

#[tauri::command]
fn view_window_open(app: tauri::AppHandle, view: String) -> bool {
    view_label(&view).map_or(false, |l| app.get_webview_window(l).is_some())
}

/// Setzt die UI-Skalierung für alle offenen Fenster.
#[tauri::command]
fn set_ui_zoom(app: tauri::AppHandle, factor: f64) -> Result<(), String> {
    let f = factor.clamp(0.5, 2.0);
    *app.state::<AppState>().ui_zoom.lock().unwrap_or_else(|p| p.into_inner()) = f;
    for w in app.webview_windows().values() {
        w.set_zoom(f).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn get_ui_zoom(app: tauri::AppHandle) -> f64 {
    *app.state::<AppState>().ui_zoom.lock().unwrap_or_else(|p| p.into_inner())
}

/// Öffnet eine exportierte Datei oder einen Ordner mit der Standardanwendung (Explorer, Browser).
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("Nicht gefunden: {path}"));
    }
    std::process::Command::new("explorer").arg(p).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Beendet die Anwendung, nachdem die Oberfläche ungespeicherte Änderungen behandelt hat.
#[tauri::command]
fn confirm_close(app: tauri::AppHandle) {
    app.exit(0);
}

/// Leitet UI-Ereignisse (Package-Auswahl usw.) an alle Fenster weiter.
#[tauri::command]
fn broadcast(app: tauri::AppHandle, event: String, payload: Value) -> Result<(), String> {
    if !matches!(event.as_str(), "pkg-focus" | "open-package") {
        return Err("Ereignis nicht erlaubt".into());
    }
    app.emit(&event, payload).map_err(|e| e.to_string())
}

/// Nimmt eine per Drag & Drop / Datei-Input hochgeladene Datei als Rohdaten entgegen und legt sie
/// in einem temporären Upload-Ordner ab. Gibt den Dateipfad zurück (anschließend `/api/import_paths`).
#[tauri::command]
fn stage_upload(request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(data) = request.body() else {
        return Err("Erwartet binäre Dateidaten".into());
    };
    let header = |k: &str| request.headers().get(k).and_then(|v| v.to_str().ok()).map(api::url_decode);
    let name = header("x-file-name").unwrap_or_else(|| "upload.bin".into());
    let name = std::path::Path::new(&name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "upload.bin".into());
    let batch: String = header("x-batch").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let dir = upload_root().join(if batch.is_empty() { "default".into() } else { batch });
    std::fs::create_dir_all(&dir).map_err(|e| format!("Upload-Ordner nicht anlegbar: {e}"))?;
    let path = dir.join(name);
    std::fs::write(&path, data).map_err(|e| format!("Upload fehlgeschlagen: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = std::fs::remove_dir_all(upload_root());
    tauri::Builder::default()
        .manage(AppState {
            engine: Mutex::new(StencilEngine::new(0.120, engine::resolve_data_root())),
            ui_zoom: Mutex::new(1.0),
        })
        .on_window_event(|window, event| {
            // Hauptfenster schließen bei ungespeicherten Änderungen: erst nachfragen (Oberfläche)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let dirty = window.app_handle().state::<AppState>().engine.lock().map(|e| e.dirty).unwrap_or(false);
                    if dirty {
                        api.prevent_close();
                        let _ = window.emit("close-requested", ());
                    }
                }
            }
            if let tauri::WindowEvent::Destroyed = event {
                let label = window.label().to_string();
                if label == "main" {
                    // Hauptfenster geschlossen → auch Zusatzfenster schließen
                    for (l, w) in window.app_handle().webview_windows() {
                        if l != "main" {
                            let _ = w.close();
                        }
                    }
                } else {
                    let _ = window.app_handle().emit("view-closed", label);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            api_request,
            stage_upload,
            open_view_window,
            view_window_open,
            set_ui_zoom,
            get_ui_zoom,
            broadcast,
            confirm_close,
            open_path
        ])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der SMT StencilCraft Desktop Anwendung");
}
