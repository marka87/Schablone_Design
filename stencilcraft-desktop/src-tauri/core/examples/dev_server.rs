//! Entwicklungsserver: Oberfläche direkt aus `templates/` + `static/` im Browser, `/api/...` an die Engine.
//! Ohne Tauri-Build testen: `cargo run -p stencil_core --example dev_server -- [port]`, dann http://127.0.0.1:8765
//! Datensatz laden (Browser-Konsole): fetch("/api/import_paths",{method:"POST",body:JSON.stringify({paths:["C:/…/job.tgz"]})})
//! Dateidialoge gibt es hier nicht (gelten als abgebrochen).

use serde_json::Value;
use stencil_core::api::{dispatch, NoDialogs};
use stencil_core::engine::{resolve_data_root, StencilEngine};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;

fn main() {
    let port = std::env::args().nth(1).unwrap_or_else(|| "8765".into());
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let engine = Mutex::new(StencilEngine::new(0.12, resolve_data_root()));
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).expect("Port belegt");
    println!("StencilCraft dev server: http://127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (method, url) = (parts.next().unwrap_or("GET").to_string(), parts.next().unwrap_or("/").to_string());
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0; len];
        let _ = reader.read_exact(&mut body);
        let (status, ctype, bytes) = if url.starts_with("/api/") {
            let json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let (st, v) = dispatch(&engine, &method, &url, &json, &NoDialogs);
            (st, "application/json", v.to_string().into_bytes())
        } else {
            let path = url.split('?').next().unwrap_or("/");
            let file = if path == "/" { repo.join("templates/index.html") } else { repo.join(path.trim_start_matches('/')) };
            let ctype = match file.extension().and_then(|e| e.to_str()) {
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("html") => "text/html; charset=utf-8",
                _ => "application/octet-stream",
            };
            // nur Dateien unterhalb von templates/ und static/
            let allowed = path == "/" || path.starts_with("/static/") && !path.contains("..");
            match std::fs::read(&file).ok().filter(|_| allowed) {
                Some(b) => (200, ctype, b),
                None => (404, "text/plain", b"not found".to_vec()),
            }
        };
        let mut out = &stream;
        let _ = write!(out, "HTTP/1.1 {status} OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
        let _ = out.write_all(&bytes);
    }
}
