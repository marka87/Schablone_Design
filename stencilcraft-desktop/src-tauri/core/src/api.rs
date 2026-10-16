//! HTTP-ähnlicher Router: bildet sämtliche Flask-Routen der Web-App (`app.py`) 1:1 nach.
//! Die unveränderte Web-Oberfläche ruft `/api/...` auf; die Desktop-Bridge leitet diese Aufrufe
//! per Tauri-IPC hierher weiter.

use crate::engine::dfm::{calculate_pip, calculate_transfer_calculator};
use crate::engine::types::{r2, r3, r4, val_bool, val_f64, val_str};
use crate::engine::{StencilEngine, NO_DATASET};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub type Response = (u16, Value);

fn ok(v: Value) -> Response {
    (200, v)
}

fn err(status: u16, msg: impl Into<String>) -> Response {
    (status, json!({"status": "error", "message": msg.into()}))
}

/// Prozent-Dekodierung eines Pfadsegments (encodeURIComponent).
pub fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Body<'a>(&'a Map<String, Value>);

impl Body<'_> {
    fn f(&self, k: &str, d: f64) -> f64 {
        self.0.get(k).and_then(val_f64).unwrap_or(d)
    }
    fn i(&self, k: &str, d: i64) -> i64 {
        self.0.get(k).and_then(val_f64).map(|v| v as i64).unwrap_or(d)
    }
    fn b(&self, k: &str, d: bool) -> bool {
        self.0.get(k).and_then(val_bool).unwrap_or(d)
    }
    fn s(&self, k: &str) -> Option<String> {
        self.0.get(k).and_then(val_str)
    }
}

fn query_map(q: &str) -> Map<String, Value> {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (url_decode(k), Value::String(url_decode(v)))
        })
        .collect()
}

/// Native Dateidialoge (von der Desktop-Schicht bereitgestellt).
pub trait Dialogs {
    fn pick_files(&self) -> Option<Vec<PathBuf>>;
    fn pick_folder(&self) -> Option<PathBuf>;
}

/// Ohne GUI (Tests/Headless): Dialoge gelten als abgebrochen.
pub struct NoDialogs;

impl Dialogs for NoDialogs {
    fn pick_files(&self) -> Option<Vec<PathBuf>> {
        None
    }
    fn pick_folder(&self) -> Option<PathBuf> {
        None
    }
}

/// Einstiegspunkt: Dialog-Routen öffnen den nativen Dialog ohne Engine-Sperre.
pub fn dispatch(engine: &Mutex<StencilEngine>, method: &str, url: &str, body: &Value, dialogs: &dyn Dialogs) -> Response {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let empty = Map::new();
    let body_map = body.as_object().unwrap_or(&empty);

    match (method, path) {
        ("POST", "/api/open_windows_file_dialog") => {
            let Some(files) = dialogs.pick_files().filter(|f| !f.is_empty()) else {
                return ok(json!({"status": "cancelled", "message": "Keine Datei ausgewählt"}));
            };
            let paths: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
            let name = if paths.len() == 1 { base(&paths[0]) } else { format!("Importiert ({} Dateien)", paths.len()) };
            let mut e = lock(engine);
            import_response(&mut e, &paths, Body(body_map), name, None)
        }
        ("POST", "/api/open_windows_folder_dialog") => {
            let Some(folder) = dialogs.pick_folder() else {
                return ok(json!({"status": "cancelled", "message": "Kein Ordner ausgewählt"}));
            };
            let f = folder.to_string_lossy().to_string();
            let name = format!("Ordner ({})", base(&f));
            let mut e = lock(engine);
            import_response(&mut e, &[f.clone()], Body(body_map), name, Some(f))
        }
        _ => {
            let mut e = lock(engine);
            let q = query_map(query);
            handle(&mut e, method, path, body_map, &q)
        }
    }
}

fn lock(m: &Mutex<StencilEngine>) -> std::sync::MutexGuard<'_, StencilEngine> {
    // Nach einem Panic in einem früheren Aufruf weiterarbeiten statt die App unbrauchbar zu machen
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn base(p: &str) -> String {
    Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string())
}

fn import_response(e: &mut StencilEngine, paths: &[String], b: Body, name: String, folder: Option<String>) -> Response {
    let side = b.s("side").unwrap_or_else(|| "top".into());
    let clear = b.b("clear_previous", true);
    match e.import_files_or_zip(paths, &side, clear) {
        Ok(count) => {
            e.dataset_name = name;
            let mut r = json!({
                "status": "ok",
                "count": count,
                "dataset_name": e.dataset_name,
                "layers": e.layers_summary(),
                "stats": e.get_panel_statistics(),
            });
            if let Some(f) = folder {
                r["folder"] = json!(f);
            }
            ok(r)
        }
        Err(msg) => err(500, msg),
    }
}

fn stats(e: &StencilEngine) -> Value {
    e.get_panel_statistics()
}

pub fn handle(e: &mut StencilEngine, method: &str, path: &str, body: &Map<String, Value>, query: &Map<String, Value>) -> Response {
    let b = Body(body);
    let seg: Vec<String> = path.trim_start_matches('/').split('/').map(url_decode).collect();
    let seg: Vec<&str> = seg.iter().map(String::as_str).collect();

    match (method, seg.as_slice()) {
        ("POST", ["api", "load_dataset"]) => {
            let key = b.s("dataset").unwrap_or_else(|| "top".into());
            let side = if key == "bot" { "bot" } else { "top" };
            let dir = e.data_root.join("Bsp").join("test_extracted");
            let paste = dir.join(format!("stencilOutput_{side}_panel_120.ger"));
            if !paste.exists() {
                return err(404, format!("Demo-Datensatz nicht gefunden: {}", paste.display()));
            }
            let zone = dir.join(format!("stencilOutput_{side}_zones_{}.ger", if side == "top" { "top" } else { "bottom" }));
            let profile = dir.join(format!("stencilOutput_{side}_profile.ger"));
            let fid = dir.join(format!("stencilOutput_{side}_fiducials_{}.ger", if side == "top" { "top" } else { "bottom" }));
            let pad = e.data_root.join("Bsp").join(if side == "top" { "04_smt.ger" } else { "28_smb.ger" });
            if let Err(m) = e.load_panel_dataset(&paste, Some(&zone), Some(&pad), Some(&profile), Some(&fid)) {
                return err(500, m);
            }
            e.current_side = side.into();
            e.dataset_name = format!("{} Side Panel ({}µm)", if side == "top" { "Top" } else { "Bottom" }, (e.base_thickness * 1000.0).round());
            ok(json!({"status": "ok", "dataset_name": e.dataset_name, "layers": e.layers_summary(), "stats": stats(e)}))
        }
        ("GET", ["api", "stats"]) => ok(json!({"dataset_name": e.dataset_name, "stats": stats(e)})),
        ("GET", ["api", "apertures"]) => ok(apertures_payload(e)),
        ("GET", ["api", "aperture", id]) => {
            let Some(ap) = id.parse::<usize>().ok().and_then(|i| e.apertures.get(i)) else {
                return (404, json!({"error": "Not found"}));
            };
            let ipc = e.calculate_aperture_ipc(ap);
            let identical = e.apertures.iter().filter(|a| a.package_group == ap.package_group).count();
            ok(json!({
                "id": ap.id,
                "cx": r4(ap.cx),
                "cy": r4(ap.cy),
                "width": r4(ap.width),
                "height": r4(ap.height),
                "area": (ap.area * 1e5).round() / 1e5,
                "perimeter": r4(ap.perimeter),
                "shape": ap.shape_type,
                "package_group": ap.package_group,
                "package_name": ap.package_name,
                "comp_id": ap.component_id,
                "pin_number": ap.pin_number,
                "facing_dir": ap.facing_dir,
                "manual": ap.is_manually_overridden,
                "identical_count": identical,
                "ipc": ipc,
                "custom_slices": ap.custom_slices,
                "points": ap.points,
            }))
        }
        ("POST", ["api", "slice"]) => {
            let n = e.apply_thermal_slicing(b.i("id", -1), b.i("grid_x", 2), b.i("grid_y", 2), b.f("web_width", 0.25), b.f("edge_reduction", 0.05), b.b("apply_all", false));
            ok(json!({"status": "ok", "modified_count": n, "stats": stats(e)}))
        }
        ("POST", ["api", "reduce"]) => {
            let n = e.apply_reduction(b.i("id", -1), b.f("percent", 10.0), b.b("apply_all", false));
            ok(json!({"status": "ok", "modified_count": n, "stats": stats(e)}))
        }
        ("POST", ["api", "reset_aperture"]) => {
            e.reset_aperture_modification(b.i("id", -1), b.b("apply_all", false));
            ok(json!({"status": "ok", "stats": stats(e)}))
        }
        ("POST", ["api", "set_ipc_mode"]) => {
            let mode = b.s("mode").unwrap_or_else(|| "standard".into());
            e.set_ipc_mode(&mode, b.f("min_ar", 0.66), b.f("min_aspect", 1.50));
            ok(json!({"status": "ok", "stats": stats(e)}))
        }
        ("POST", ["api", "zone"]) => {
            let z = e.add_zone(b.f("x0", 0.0), b.f("y0", 0.0), b.f("x1", 0.0), b.f("y1", 0.0), b.f("thickness", 0.100), &b.s("name").unwrap_or_default());
            ok(json!({"status": "ok", "zone": z.to_json(), "stats": stats(e)}))
        }
        ("DELETE", ["api", "zone", id]) => {
            e.delete_zone(id);
            ok(json!({"status": "ok", "stats": stats(e)}))
        }
        ("PUT" | "POST", ["api", "zone", id]) => {
            e.update_zone(id, b.f("thickness", 0.100), b.s("name"));
            ok(json!({"status": "ok", "stats": stats(e)}))
        }
        ("POST", ["api", "elongate"]) => {
            let n = e.apply_elongated_narrowing(b.i("id", -1), b.f("width_reduction", 20.0), b.f("length_usage", 95.0), b.f("corner_radius", 0.08), b.b("apply_all", false));
            ok(json!({"status": "ok", "modified_count": n, "stats": stats(e)}))
        }
        ("POST", ["api", "calculator"]) => {
            let th: Vec<f64> = body
                .get("thicknesses")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(val_f64).collect())
                .unwrap_or_else(|| vec![80.0, 100.0, 110.0, 120.0, 150.0]);
            ok(calculate_transfer_calculator(b.f("width", 11.8), b.f("height", 11.8), &b.s("unit").unwrap_or_else(|| "mil".into()), &th))
        }
        ("POST", ["api", "load_zip"]) => {
            let Some(p) = b.s("path").filter(|p| !p.is_empty()) else {
                return err(400, "Kein Pfad angegeben");
            };
            let side = b.s("side").unwrap_or_else(|| "top".into());
            match e.import_files_or_zip(&[p.clone()], &side, true) {
                Ok(_) => {
                    e.dataset_name = format!("ZIP ({} - {})", base(&p), side.to_uppercase());
                    ok(json!({"status": "ok", "dataset_name": e.dataset_name, "layers": e.layers_summary(), "stats": stats(e)}))
                }
                Err(m) => err(400, m),
            }
        }
        // Desktop: Upload-Dateien werden von der Bridge in ein Temp-Verzeichnis gelegt und als Pfade übergeben
        ("POST", ["api", "import_paths"]) => {
            let paths: Vec<String> = body.get("paths").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            if paths.is_empty() {
                return err(400, "Keine Dateien hochgeladen");
            }
            let name = if paths.len() == 1 { base(&paths[0]) } else { format!("Importiert ({} Dateien)", paths.len()) };
            import_response(e, &paths, b, name, None)
        }
        ("GET", ["api", "layers"]) => ok(json!({"status": "ok", "dataset_name": e.dataset_name, "layers": e.layers_summary()})),
        ("GET", ["api", "layer_primitives"]) => {
            let all = query.get("all").and_then(|v| v.as_str()).map_or(true, |v| v == "1");
            let mut data = Map::new();
            for l in &e.layers {
                if all || l.visible {
                    data.insert(
                        l.id.clone(),
                        json!({"id": l.id, "name": l.name, "color": l.color, "opacity": l.opacity, "type": l.layer_type, "visible": l.visible, "prims": l.primitives}),
                    );
                }
            }
            ok(json!({"status": "ok", "layers": data}))
        }
        ("POST", ["api", "layer", id, "toggle"]) => {
            let vis = body.get("visible").filter(|v| !v.is_null()).and_then(val_bool);
            match e.layer_mut(id) {
                Some(l) => {
                    l.visible = vis.unwrap_or(!l.visible);
                    ok(json!({"status": "ok", "layer_id": id, "visible": l.visible}))
                }
                None => err(404, "Layer nicht gefunden"),
            }
        }
        ("POST", ["api", "layer", id, "update"]) => {
            e.update_layer_settings(id, body.get("visible").filter(|v| !v.is_null()).and_then(val_bool), b.s("color"), body.get("opacity").and_then(val_f64), b.s("type"));
            ok(json!({"status": "ok"}))
        }
        ("DELETE", ["api", "layer", id]) => {
            e.remove_layer(id);
            ok(json!({"status": "ok", "layers": e.layers_summary()}))
        }
        ("POST", ["api", "layers", "toggle_all"]) => {
            let v = b.b("visible", true);
            for l in e.layers.iter_mut() {
                l.visible = v;
            }
            ok(json!({"status": "ok", "layers": e.layers_summary()}))
        }
        ("POST", ["api", "export"]) => {
            let req = b.s("side").unwrap_or_default().to_lowercase();
            let mut side = if matches!(req.as_str(), "top" | "bot" | "bottom") { req } else { e.current_side.clone() };
            if side == "bottom" {
                side = "bot".into();
            }
            let dir = b.s("target_dir").map(PathBuf::from).unwrap_or_else(|| e.export_dir());
            let stencil = dir.join(format!("stencil_{side}.ger"));
            let zones = dir.join(format!("stencil_{side}_zones.ger"));
            if let Err(m) = e.export_modified_gerber(&stencil).and_then(|_| e.export_zones_gerber(&zones)) {
                return err(500, m);
            }
            ok(json!({
                "status": "ok",
                "side": side,
                "stencil_file": stencil.to_string_lossy(),
                "zones_file": zones.to_string_lossy(),
                "export_dir": dir.to_string_lossy(),
                "message": format!("Separate Schablonen-Dateien für {} exportiert: {}", side.to_uppercase(), stencil.display()),
            }))
        }
        ("GET", ["api", "packages"]) => {
            let cat = query.get("category").and_then(|v| v.as_str()).unwrap_or("");
            let search = query.get("search").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let order = |c: &str| match c {
                "SMD Passives" => 0,
                "ICs & Transistors" => 1,
                "BGA & Arrays" => 2,
                "Other" => 3,
                _ => 4,
            };
            let mut list: Vec<_> = e
                .packages
                .values()
                .filter(|p| cat.is_empty() || p.category == cat)
                .filter(|p| search.is_empty() || p.name.to_lowercase().contains(&search))
                .collect();
            list.sort_by(|a, b| order(&a.category).cmp(&order(&b.category)).then(a.name.cmp(&b.name)));
            ok(json!({"current_customer": e.current_customer, "packages": list.iter().map(|p| p.to_json()).collect::<Vec<_>>()}))
        }
        ("GET", ["api", "package", name]) => ok(e.get_package_preview_data(name)),
        ("POST", ["api", "package", name, "rule"]) => {
            let target = e.resolve_alias(name);
            if let Some(p) = e.packages.get_mut(&target) {
                p.rule.apply_map(body);
            }
            ok(json!({"status": "ok", "preview": e.get_package_preview_data(name)}))
        }
        ("POST", ["api", "package", name, "generate"]) => {
            let n = e.apply_package_rule(name, body);
            ok(json!({"status": "ok", "package": name, "modified_count": n, "stats": stats(e)}))
        }
        ("POST", ["api", "pad", id, "override"]) => {
            let s = e.override_single_pad(id.parse().unwrap_or(-1), body);
            ok(json!({"status": if s { "ok" } else { "error" }, "stats": stats(e)}))
        }
        ("POST", ["api", "pad", id, "reset"]) => {
            let s = e.reset_single_pad(id.parse().unwrap_or(-1));
            ok(json!({"status": if s { "ok" } else { "error" }, "stats": stats(e)}))
        }
        ("POST", ["api", "close_dataset"]) => {
            e.close_dataset();
            ok(json!({"status": "ok", "dataset_name": NO_DATASET, "layers": e.layers_summary(), "stats": stats(e)}))
        }
        (m @ ("POST" | "DELETE"), ["api", "package", name, "pin", pin, "override"]) => {
            let Ok(pin) = pin.parse::<i64>() else { return err(400, "Ungültige Pin-Nummer") };
            let s = if m == "POST" { e.override_package_pin(name, pin, body.clone()) } else { e.reset_package_pin(name, pin) };
            ok(json!({"status": if s { "ok" } else { "error" }, "preview": e.get_package_preview_data(name), "stats": stats(e)}))
        }
        ("POST", ["api", "package", name, "rename"]) => {
            let new = b.s("new_name").unwrap_or_default().trim().to_string();
            if new.is_empty() {
                return err(400, "Neuer Name darf nicht leer sein");
            }
            let s = e.rename_package(name, &new);
            ok(json!({"status": if s { "ok" } else { "error" }, "old_name": name, "new_name": new, "packages": e.packages.values().map(|p| p.to_json()).collect::<Vec<_>>()}))
        }
        ("GET", ["api", "libraries"]) => ok(json!({"libraries": e.list_customer_libraries(), "current_customer": e.current_customer})),
        ("POST", ["api", "library", "load"]) => {
            let c = b.s("customer").unwrap_or_else(|| "Standard".into());
            let s = e.load_customer_library(&c);
            ok(json!({"status": if s { "ok" } else { "error" }, "customer": e.current_customer, "packages": e.packages.values().map(|p| p.to_json()).collect::<Vec<_>>()}))
        }
        ("POST", ["api", "library", "save"]) => {
            let c = b.s("customer").unwrap_or_else(|| e.current_customer.clone());
            match e.save_customer_library(&c) {
                Ok(p) => ok(json!({"status": "ok", "customer": e.current_customer, "path": p})),
                Err(m) => err(500, m),
            }
        }
        ("GET" | "POST", ["api", "dfm", "bridge_check"]) => {
            let mb = if method == "POST" { b.f("min_bridge", e.min_bridge_width) } else { Body(query).f("min_bridge", e.min_bridge_width) };
            ok(e.check_stencil_bridges(mb))
        }
        ("POST", ["api", "pip", "calculate"]) => {
            let st = b.f("stencil_thick", 0.0);
            let t = if st > 0.0 { st } else { e.base_thickness };
            ok(calculate_pip(
                b.f("d_drill", 1.0),
                &b.s("pin_shape").unwrap_or_else(|| "round".into()),
                b.f("pin_w", 0.6),
                b.f("pin_h", 0.6),
                b.f("h_pcb", 1.6),
                b.f("metal_ratio", 0.50),
                b.f("v_meniscus", 0.0),
                t,
                &b.s("overprint_shape").unwrap_or_else(|| "rectangle".into()),
            ))
        }
        ("POST", ["api", "pip", "apply"]) => {
            let id = b.i("ap_id", -1);
            let empty = Map::new();
            let pip = body.get("pip_data").and_then(|v| v.as_object()).unwrap_or(&empty);
            let s = e.apply_pip_aperture(id, pip);
            ok(json!({"status": if s { "ok" } else { "error" }, "ap_id": id, "stats": stats(e)}))
        }
        ("POST", ["api", "auto_classify"]) => {
            let c = e.auto_classify_and_apply_rules();
            ok(json!({"status": "ok", "classification": c, "panel_stats": stats(e), "packages": e.packages.values().map(|p| p.to_json()).collect::<Vec<_>>()}))
        }
        ("POST", ["api", "set_paste_type"]) => {
            let t = e.set_paste_type(&b.s("paste_type").unwrap_or_else(|| "type4".into()));
            ok(json!({"status": "ok", "paste_type": t, "stats": stats(e)}))
        }
        ("POST", ["api", "set_thickness"]) => {
            let t = b.f("thickness", e.base_thickness);
            if t > 0.0 && t < 1.0 {
                e.base_thickness = t;
                let base = e.base_thickness;
                for z in e.zones.iter_mut() {
                    z.is_step_down = z.thickness < base;
                }
            }
            ok(json!({"status": "ok", "base_thickness": e.base_thickness, "stats": stats(e)}))
        }
        _ => err(404, format!("Unbekannte API-Route: {method} {path}")),
    }
}

fn apertures_payload(e: &StencilEngine) -> Value {
    let ipcs = e.all_ipc();
    let aps: Vec<Value> = e
        .apertures
        .iter()
        .zip(ipcs)
        .map(|(ap, ipc)| {
            let mut v = json!({
                "id": ap.id,
                "cx": r3(ap.cx),
                "cy": r3(ap.cy),
                "w": r3(ap.width),
                "h": r3(ap.height),
                "shape": ap.shape_type,
                "ar": ipc.area_ratio,
                "aspect": ipc.aspect_ratio,
                "status": ipc.status,
                "thick": ipc.effective_thickness,
                "zone_id": ipc.zone_id,
                "pkg": ap.package_group,
                "pkg_name": ap.package_name,
                "comp_id": ap.component_id,
                "manual": ap.is_manually_overridden,
                "warn": ipc.clearance_warn,
                "grain_warn": ipc.grain_warn,
                "grain_msg": ipc.grain_msg,
                "sliced": ap.custom_slices.as_ref().map_or(false, |s| !s.is_empty()),
            });
            if let Some(s) = ap.custom_slices.as_ref() {
                v["slices"] = json!(s);
            } else if ap.shape_type == "polygon" && !ap.points.is_empty() {
                v["pts"] = json!(ap.points);
            }
            v
        })
        .collect();

    let profile: Vec<Value> = e.profile_paths.iter().take(5000).map(|s| json!([[s[0].0, s[0].1], [s[1].0, s[1].1]])).collect();
    let components: Vec<Value> = e.components.iter().map(|c| serde_json::to_value(c).unwrap_or(Value::Null)).collect();
    let pads: Vec<Value> = e.pads.iter().take(2000).map(|p| json!({"x0": p.x0, "y0": p.y0, "x1": p.x1, "y1": p.y1, "w": p.w, "h": p.h})).collect();
    json!({
        "bounds": e.bounds.iter().map(|b| r2(*b)).collect::<Vec<_>>(),
        "apertures": aps,
        "zones": e.zones.iter().map(|z| z.to_json()).collect::<Vec<_>>(),
        "pads": pads,
        "components": components,
        "fiducials": e.fiducials.iter().map(|f| json!([f.0, f.1, f.2])).collect::<Vec<_>>(),
        "profile": profile,
        "paste_type": e.paste_type,
        "min_bridge": e.min_bridge_width,
        "bridge_violations": &e.bridge_violations[..e.bridge_violations.len().min(100)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::StencilEngine;

    fn engine_with_chips() -> StencilEngine {
        let mut e = StencilEngine::new(0.12, std::env::temp_dir().join("stencilcraft_test_root"));
        let mut g = String::from("%FSLAX46Y46*%%MOMM*%%ADD10R,1.2X1.3*%%ADD11R,4.5X4.5*%D10*\n");
        for i in 0..5 {
            let x = 10.0 + i as f64 * 5.0;
            g.push_str(&format!("X{}Y{}D03*\nX{}Y{}D03*\n", ((x - 0.851) * 1e6) as i64, 10_000_000, ((x + 0.851) * 1e6) as i64, 10_000_000));
        }
        g.push_str("D11*X50000000Y50000000D03*M02*");
        let path = std::env::temp_dir().join("stencilcraft_test_top_paste.ger");
        std::fs::write(&path, g).unwrap();
        e.import_files_or_zip(&[path.to_string_lossy().to_string()], "top", true).unwrap();
        e
    }

    #[test]
    fn route_roundtrip_and_package_detection() {
        let mut e = engine_with_chips();
        assert_eq!(e.apertures.len(), 11);
        assert_eq!(e.packages["0805"].component_count, 5);
        let empty = Map::new();
        let (st, v) = handle(&mut e, "GET", "/api/apertures", &empty, &empty);
        assert_eq!(st, 200);
        assert_eq!(v["apertures"].as_array().unwrap().len(), 11);

        let rule = json!({"rule_type": "house_builder", "house_cut_pct": 30.0});
        let (_, v) = handle(&mut e, "POST", "/api/package/0805/generate", rule.as_object().unwrap(), &empty);
        assert_eq!(v["modified_count"], 10);
        assert!(e.apertures[0].custom_slices.as_ref().unwrap()[0].pts.is_some());

        let body = json!({"id": 10, "grid_x": 3, "grid_y": 3, "web_width": 0.3});
        let (_, v) = handle(&mut e, "POST", "/api/slice", body.as_object().unwrap(), &empty);
        assert_eq!(v["modified_count"], 1);
        assert_eq!(e.apertures[10].custom_slices.as_ref().unwrap().len(), 9);

        // Slices desselben Pads: erst unter 0.08 mm kritisch (wie Web-App)
        let (_, v) = handle(&mut e, "POST", "/api/dfm/bridge_check", json!({"min_bridge": 0.35}).as_object().unwrap(), &empty);
        assert_eq!(v["count"], 0);
        // Chip-Pads (Abstand ca. 0.55 mm nach HouseBuilder) unter 0.8 mm Grenzwert → 5 Verstöße
        let (_, v) = handle(&mut e, "POST", "/api/dfm/bridge_check", json!({"min_bridge": 0.8}).as_object().unwrap(), &empty);
        assert_eq!(v["count"], 5, "{v}");
        let d = v["violations"][0]["dist"].as_f64().unwrap();
        assert!(d > 0.5 && d < 0.6, "Polygonabstand {d}");

        let (st, _) = handle(&mut e, "GET", "/api/package/0805", &empty, &empty);
        assert_eq!(st, 200);
        let (st, _) = handle(&mut e, "GET", "/api/unknown", &empty, &empty);
        assert_eq!(st, 404);
    }

    #[test]
    fn export_writes_parsable_gerber() {
        let mut e = engine_with_chips();
        let dir = std::env::temp_dir().join("stencilcraft_export_test");
        let empty = Map::new();
        handle(&mut e, "POST", "/api/reduce", json!({"id": 0, "percent": 20, "apply_all": true}).as_object().unwrap(), &empty);
        let body = json!({"side": "top", "target_dir": dir.to_string_lossy()});
        let (st, v) = handle(&mut e, "POST", "/api/export", body.as_object().unwrap(), &empty);
        assert_eq!(st, 200, "{v}");
        let text = std::fs::read_to_string(dir.join("stencil_top.ger")).unwrap();
        let back = crate::engine::gerber::parse_gerber(&text).paste_apertures();
        assert_eq!(back.len(), 11);
    }

    #[test]
    fn url_decoding() {
        assert_eq!(url_decode("SOT-23%20A%2FB"), "SOT-23 A/B");
        assert_eq!(url_decode("100%"), "100%");
    }
}
