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
use tauri::{Manager, State};

pub struct AppState {
    pub engine: Mutex<StencilEngine>,
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
}

fn upload_root() -> PathBuf {
    std::env::temp_dir().join("stencilcraft_upload")
}

/// Führt einen API-Aufruf der Web-Oberfläche aus (Methode, URL inkl. Query, JSON-Body).
/// Läuft in einem Hintergrund-Thread, damit Import/DFM die Oberfläche nicht blockieren.
#[tauri::command]
async fn api_request(app: tauri::AppHandle, method: String, url: String, body: Option<Value>) -> Result<ApiResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
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
        ApiResponse { status, body }
    })
    .await
    .map_err(|e| e.to_string())
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
        .manage(AppState { engine: Mutex::new(StencilEngine::new(0.120, engine::resolve_data_root())) })
        .invoke_handler(tauri::generate_handler![api_request, stage_upload])
        .run(tauri::generate_context!())
        .expect("Fehler beim Starten der SMT StencilCraft Desktop Anwendung");
}
