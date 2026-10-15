"""
SMT Stencil Assistant - Lokaler Server
Leichtgewichtig, extrem performant, sofort startklar.
"""

import os
import sys
from flask import Flask, render_template, jsonify, request, send_file
from stencil_engine import StencilEngine

app = Flask(__name__, static_folder="static", template_folder="templates")

# Globale Engine-Instanz
engine = StencilEngine(base_thickness=0.120)
current_dataset_name = "Kein Datensatz geladen"

DATASET_PATHS = {
    "top": {
        "name": "Top Side Panel (120µm)",
        "paste": r"A:\Schablone\Bsp\test_extracted\stencilOutput_top_panel_120.ger",
        "zone": r"A:\Schablone\Bsp\test_extracted\stencilOutput_top_zones_top.ger",
        "profile": r"A:\Schablone\Bsp\test_extracted\stencilOutput_top_profile.ger",
        "fiducial": r"A:\Schablone\Bsp\test_extracted\stencilOutput_top_fiducials_top.ger",
        "pad": r"A:\Schablone\Bsp\04_smt.ger"
    },
    "bot": {
        "name": "Bottom Side Panel (120µm)",
        "paste": r"A:\Schablone\Bsp\test_extracted\stencilOutput_bot_panel_120.ger",
        "zone": r"A:\Schablone\Bsp\test_extracted\stencilOutput_bot_zones_bottom.ger",
        "profile": r"A:\Schablone\Bsp\test_extracted\stencilOutput_bot_profile.ger",
        "fiducial": r"A:\Schablone\Bsp\test_extracted\stencilOutput_bot_fiducials_bottom.ger",
        "pad": r"A:\Schablone\Bsp\28_smb.ger"
    }
}


def load_dataset_by_key(key: str):
    global current_dataset_name
    info = DATASET_PATHS.get(key, DATASET_PATHS["top"])
    current_dataset_name = info["name"]
    engine.load_panel_dataset(
        paste_path=info["paste"],
        zone_path=info.get("zone"),
        pad_path=info.get("pad"),
        profile_path=info.get("profile"),
        fiducial_path=info.get("fiducial")
    )


# Startet standardmäßig mit leerem Workspace (Benutzer öffnet ODB++/Gerber über Button)



@app.route("/")
def index():
    return render_template("index.html", dataset_name=current_dataset_name)


@app.route("/api/load_dataset", methods=["POST"])
def api_load_dataset():
    data = request.json or {}
    key = data.get("dataset", "top")
    load_dataset_by_key(key)
    return jsonify({
        "status": "ok",
        "dataset_name": current_dataset_name,
        "layers": engine.get_layers_summary(),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/stats", methods=["GET"])
def api_stats():
    return jsonify({
        "dataset_name": current_dataset_name,
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/apertures", methods=["GET"])
def api_apertures():
    """Gibt komprimierte Vektordaten aller Aperturen für das HTML5 Canvas zurück."""
    payload = []
    for ap in engine.apertures:
        ipc = engine.calculate_aperture_ipc(ap)
        # Kompaktes Datenformat zur Minimierung des Netzwerk-Overheads
        item_data = {
            "id": ap.id,
            "cx": round(ap.cx, 3),
            "cy": round(ap.cy, 3),
            "w": round(ap.width, 3),
            "h": round(ap.height, 3),
            "shape": ap.shape_type,
            "ar": ipc["area_ratio"],
            "aspect": ipc["aspect_ratio"],
            "status": ipc["status"],
            "thick": ipc["effective_thickness"],
            "zone_id": ipc["zone_id"],
            "pkg": ap.package_group,
            "pkg_name": ap.package_name,
            "comp_id": ap.component_id,
            "manual": ap.is_manually_overridden,
            "warn": ipc["clearance_warn"],
            "sliced": bool(ap.custom_slices and len(ap.custom_slices) > 0)
        }
        if ap.custom_slices:
            item_data["slices"] = ap.custom_slices
        elif ap.shape_type == "polygon" and len(ap.points) <= 12:
            item_data["pts"] = ap.points

        payload.append(item_data)

    return jsonify({
        "bounds": [round(b, 2) for b in engine.bounds],
        "apertures": payload,
        "zones": [z.to_dict() for z in engine.zones],
        "pads": engine.pads[:2000],  # Kupferpads (grün)
        "components": [c.to_dict() for c in engine.components],  # Bauteile (gelb/weiß)
        "fiducials": engine.fiducials,
        "profile": engine.profile_paths[:1000]  # Erste 1000 Pfade für Kontur
    })


@app.route("/api/aperture/<int:ap_id>", methods=["GET"])
def api_get_aperture_detail(ap_id: int):
    if ap_id < 0 or ap_id >= len(engine.apertures):
        return jsonify({"error": "Not found"}), 404

    ap = engine.apertures[ap_id]
    ipc = engine.calculate_aperture_ipc(ap)
    identical_count = sum(1 for a in engine.apertures if a.package_group == ap.package_group)

    return jsonify({
        "id": ap.id,
        "cx": round(ap.cx, 4),
        "cy": round(ap.cy, 4),
        "width": round(ap.width, 4),
        "height": round(ap.height, 4),
        "area": round(ap.area, 5),
        "perimeter": round(ap.perimeter, 4),
        "shape": ap.shape_type,
        "package_group": ap.package_group,
        "identical_count": identical_count,
        "ipc": ipc,
        "custom_slices": ap.custom_slices,
        "points": ap.points
    })


@app.route("/api/slice", methods=["POST"])
def api_slice():
    data = request.json or {}
    ap_id = data.get("id", -1)
    grid_x = int(data.get("grid_x", 2))
    grid_y = int(data.get("grid_y", 2))
    web = float(data.get("web_width", 0.25))
    edge = float(data.get("edge_reduction", 0.05))
    all_identical = bool(data.get("apply_all", False))

    modified = engine.apply_thermal_slicing(ap_id, grid_x, grid_y, web, edge, all_identical)
    return jsonify({
        "status": "ok",
        "modified_count": len(modified),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/reduce", methods=["POST"])
def api_reduce():
    data = request.json or {}
    ap_id = data.get("id", -1)
    percent = float(data.get("percent", 10.0))
    all_identical = bool(data.get("apply_all", False))

    modified = engine.apply_reduction(ap_id, percent, all_identical)
    return jsonify({
        "status": "ok",
        "modified_count": len(modified),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/reset_aperture", methods=["POST"])
def api_reset_aperture():
    data = request.json or {}
    ap_id = data.get("id", -1)
    all_identical = bool(data.get("apply_all", False))

    engine.reset_aperture_modification(ap_id, all_identical)
    return jsonify({
        "status": "ok",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/set_ipc_mode", methods=["POST"])
def api_set_ipc_mode():
    data = request.json or {}
    mode = data.get("mode", "standard")
    ar = float(data.get("min_ar", 0.66))
    aspect = float(data.get("min_aspect", 1.50))
    engine.set_ipc_mode(mode, ar, aspect)
    return jsonify({
        "status": "ok",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/zone", methods=["POST"])
def api_add_zone():
    data = request.json or {}
    x0 = float(data.get("x0", 0))
    y0 = float(data.get("y0", 0))
    x1 = float(data.get("x1", 0))
    y1 = float(data.get("y1", 0))
    thick = float(data.get("thickness", 0.100))
    name = data.get("name", "")

    z = engine.add_zone(x0, y0, x1, y1, thick, name)
    return jsonify({
        "status": "ok",
        "zone": z.to_dict(),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/zone/<zone_id>", methods=["DELETE"])
def api_delete_zone(zone_id: str):
    engine.delete_zone(zone_id)
    return jsonify({
        "status": "ok",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/elongate", methods=["POST"])
def api_elongate():
    data = request.json or {}
    ap_id = data.get("id", -1)
    width_red = float(data.get("width_reduction", 20.0))
    length_use = float(data.get("length_usage", 95.0))
    corner_r = float(data.get("corner_radius", 0.08))
    all_identical = bool(data.get("apply_all", False))

    modified = engine.apply_elongated_narrowing(ap_id, width_red, length_use, corner_r, all_identical)
    return jsonify({
        "status": "ok",
        "modified_count": len(modified),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/calculator", methods=["POST"])
def api_calculator():
    data = request.json or {}
    w = float(data.get("width", 11.8))
    h = float(data.get("height", 11.8))
    unit = data.get("unit", "mil")
    thicknesses = data.get("thicknesses", [80, 100, 110, 120, 150])

    res = engine.calculate_transfer_calculator(w, h, unit, thicknesses)
    return jsonify(res)


@app.route("/api/zone/<zone_id>", methods=["PUT", "POST"])
def api_update_zone(zone_id: str):
    data = request.json or {}
    thick = float(data.get("thickness", 0.100))
    name = data.get("name")
    engine.update_zone(zone_id, thick, name)
    return jsonify({
        "status": "ok",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/load_zip", methods=["POST"])
def api_load_zip():
    data = request.json or {}
    zip_path = data.get("path", r"A:\Schablone\Bsp\test.zip")
    side = data.get("side", "top")
    try:
        engine.load_zip_archive(zip_path, side=side)
        global current_dataset_name
        current_dataset_name = f"ZIP ({os.path.basename(zip_path)} - {side.upper()})"
        return jsonify({
            "status": "ok",
            "dataset_name": current_dataset_name,
            "layers": engine.get_layers_summary(),
            "stats": engine.get_panel_statistics()
        })
    except Exception as e:
        return jsonify({"status": "error", "message": str(e)}), 400


# -------------------------------------------------------------
# Multi-Layer Management & Windows Native Import API
# -------------------------------------------------------------
@app.route("/api/open_windows_file_dialog", methods=["POST"])
def api_open_windows_file_dialog():
    """Öffnet den nativen Windows Explorer Dateidialog über isolierten Subprozess (100% thread-sicher)."""
    data = request.json or {}
    side = data.get("side", "top")
    try:
        import subprocess, json
        script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "win_dialog.py")
        proc = subprocess.run(
            [sys.executable, script, "files"],
            capture_output=True,
            text=True,
            cwd=os.path.dirname(os.path.abspath(__file__))
        )
        out = proc.stdout.strip()
        if not out:
            return jsonify({"status": "cancelled", "message": "Keine Datei ausgewählt"})

        paths = json.loads(out)
        if not paths:
            return jsonify({"status": "cancelled", "message": "Keine Datei ausgewählt"})

        clear_prev = bool(data.get("clear_previous", True))
        imported = engine.import_files_or_zip(list(paths), side=side, clear_previous=clear_prev)
        global current_dataset_name
        current_dataset_name = os.path.basename(paths[0]) if len(paths) == 1 else f"Importiert ({len(paths)} Dateien)"
        return jsonify({
            "status": "ok",
            "count": len(imported),
            "layers": engine.get_layers_summary(),
            "stats": engine.get_panel_statistics()
        })
    except Exception as e:
        import traceback
        traceback.print_exc()
        return jsonify({"status": "error", "message": str(e)}), 500


@app.route("/api/open_windows_folder_dialog", methods=["POST"])
def api_open_windows_folder_dialog():
    """Öffnet den nativen Windows Explorer Ordnerdialog über isolierten Subprozess (100% thread-sicher)."""
    data = request.json or {}
    side = data.get("side", "top")
    try:
        import subprocess, json
        script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "win_dialog.py")
        proc = subprocess.run(
            [sys.executable, script, "folder"],
            capture_output=True,
            text=True,
            cwd=os.path.dirname(os.path.abspath(__file__))
        )
        out = proc.stdout.strip()
        if not out:
            return jsonify({"status": "cancelled", "message": "Kein Ordner ausgewählt"})

        folder = json.loads(out)
        if not folder:
            return jsonify({"status": "cancelled", "message": "Kein Ordner ausgewählt"})

        clear_prev = bool(data.get("clear_previous", True))
        imported = engine.import_files_or_zip([folder], side=side, clear_previous=clear_prev)
        global current_dataset_name
        current_dataset_name = f"Ordner ({os.path.basename(folder)})"
        return jsonify({
            "status": "ok",
            "count": len(imported),
            "folder": folder,
            "layers": engine.get_layers_summary(),
            "stats": engine.get_panel_statistics()
        })
    except Exception as e:
        import traceback
        traceback.print_exc()
        return jsonify({"status": "error", "message": str(e)}), 500


@app.route("/api/upload_layers", methods=["POST"])
def api_upload_layers():
    """Browser-Upload per Drag & Drop oder HTML5 File Input."""
    files = request.files.getlist("files")
    side = request.form.get("side", "top")
    clear_prev = request.form.get("clear_previous", "true").lower() == "true"
    if not files:
        return jsonify({"status": "error", "message": "Keine Dateien hochgeladen"}), 400

    import tempfile
    temp_dir = os.path.join(tempfile.gettempdir(), "stencil_web_upload")
    os.makedirs(temp_dir, exist_ok=True)
    saved_paths = []
    for f in files:
        if f.filename:
            fp = os.path.join(temp_dir, f.filename)
            f.save(fp)
            saved_paths.append(fp)

    imported = engine.import_files_or_zip(saved_paths, side=side, clear_previous=clear_prev)
    return jsonify({
        "status": "ok",
        "count": len(imported),
        "layers": engine.get_layers_summary(),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/layers", methods=["GET"])
def api_get_layers():
    return jsonify({
        "status": "ok",
        "layers": engine.get_layers_summary()
    })


@app.route("/api/layer_primitives", methods=["GET"])
def api_get_layer_primitives():
    """Gibt alle Vektordaten der Lagen zurück (mit Caching auf Client-Seite)."""
    include_all = request.args.get("all", "1") == "1"
    data = {}
    for lid, l in engine.layers.items():
        if include_all or l.visible:
            data[lid] = {
                "id": l.id,
                "name": l.name,
                "color": l.color,
                "opacity": l.opacity,
                "type": l.layer_type,
                "visible": l.visible,
                "prims": l.primitives
            }
    return jsonify({"status": "ok", "layers": data})


@app.route("/api/layer/<layer_id>/toggle", methods=["POST"])
def api_toggle_layer(layer_id: str):
    data = request.json or {}
    visible = data.get("visible", None)
    if layer_id in engine.layers:
        if visible is not None:
            engine.layers[layer_id].visible = bool(visible)
        else:
            engine.layers[layer_id].visible = not engine.layers[layer_id].visible
        return jsonify({"status": "ok", "layer_id": layer_id, "visible": engine.layers[layer_id].visible})
    return jsonify({"status": "error", "message": "Layer nicht gefunden"}), 404


@app.route("/api/layer/<layer_id>/update", methods=["POST"])
def api_update_layer(layer_id: str):
    data = request.json or {}
    engine.update_layer_settings(
        layer_id=layer_id,
        visible=data.get("visible"),
        color=data.get("color"),
        opacity=data.get("opacity"),
        layer_type=data.get("type")
    )
    return jsonify({"status": "ok"})


@app.route("/api/layer/<layer_id>", methods=["DELETE"])
def api_delete_layer(layer_id: str):
    engine.remove_layer(layer_id)
    return jsonify({"status": "ok", "layers": engine.get_layers_summary()})


@app.route("/api/layers/toggle_all", methods=["POST"])
def api_toggle_all_layers():
    data = request.json or {}
    visible = bool(data.get("visible", True))
    for l in engine.layers.values():
        l.visible = visible
    return jsonify({"status": "ok", "layers": engine.get_layers_summary()})


@app.route("/api/export", methods=["POST"])
def api_export():
    out_dir = r"A:\Schablone\Export"
    os.makedirs(out_dir, exist_ok=True)

    data = request.json or {}
    req_side = data.get("side", "").lower()
    side = req_side if req_side in ("top", "bot", "bottom") else getattr(engine, "current_side", "top")
    if side == "bottom":
        side = "bot"

    stencil_file = os.path.join(out_dir, f"stencil_{side}.ger")
    zones_file = os.path.join(out_dir, f"stencil_{side}_zones.ger")

    engine.export_modified_gerber(stencil_file)
    engine.export_zones_gerber(zones_file)

    return jsonify({
        "status": "ok",
        "side": side,
        "stencil_file": stencil_file,
        "zones_file": zones_file,
        "message": f"Separate Schablonen-Dateien für {side.upper()} exportiert: {os.path.basename(stencil_file)}"
    })


# -------------------------------------------------------------
# Stencil Generator & Package Library API
# -------------------------------------------------------------
@app.route("/api/packages", methods=["GET"])
def api_get_packages():
    category = request.args.get("category", "")
    search = request.args.get("search", "").lower()
    packages_list = []
    for name, pkg in engine.packages.items():
        if category and pkg.category != category:
            continue
        if search and search not in name.lower():
            continue
        packages_list.append(pkg.to_dict())

    # Sortierung: SMD Passives -> ICs -> BGA -> Other
    cat_order = {"SMD Passives": 0, "ICs & Transistors": 1, "BGA & Arrays": 2, "Other": 3}
    packages_list.sort(key=lambda p: (cat_order.get(p.get("category"), 4), p["name"]))

    return jsonify({
        "current_customer": engine.current_customer,
        "packages": packages_list
    })


@app.route("/api/package/<pkg_name>", methods=["GET"])
def api_get_package_preview(pkg_name: str):
    data = engine.get_package_preview_data(pkg_name)
    return jsonify(data)


@app.route("/api/package/<pkg_name>/rule", methods=["POST"])
def api_save_package_rule(pkg_name: str):
    rule_data = request.json or {}
    target_name = engine.package_aliases.get(pkg_name, pkg_name)
    if target_name in engine.packages:
        pkg = engine.packages[target_name]
        for k, v in rule_data.items():
            if hasattr(pkg.rule, k):
                setattr(pkg.rule, k, v)
    prev = engine.get_package_preview_data(pkg_name)
    return jsonify({"status": "ok", "preview": prev})


@app.route("/api/package/<pkg_name>/generate", methods=["POST"])
def api_generate_package_stencil(pkg_name: str):
    rule_data = request.json or {}
    modified_count = engine.apply_package_rule(pkg_name, rule_data)
    return jsonify({
        "status": "ok",
        "package": pkg_name,
        "modified_count": modified_count,
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/pad/<int:ap_id>/override", methods=["POST"])
def api_pad_override(ap_id: int):
    data = request.json or {}
    success = engine.override_single_pad(ap_id, data)
    return jsonify({
        "status": "ok" if success else "error",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/pad/<int:ap_id>/reset", methods=["POST"])
def api_pad_reset(ap_id: int):
    success = engine.reset_single_pad(ap_id)
    return jsonify({
        "status": "ok" if success else "error",
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/close_dataset", methods=["POST"])
def api_close_dataset():
    global current_dataset_name
    engine.close_dataset()
    current_dataset_name = "Kein Datensatz geladen"
    return jsonify({
        "status": "ok",
        "dataset_name": current_dataset_name,
        "layers": engine.get_layers_summary(),
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/package/<pkg_name>/pin/<int:pin_num>/override", methods=["POST"])
def api_package_pin_override(pkg_name: str, pin_num: int):
    data = request.json or {}
    success = engine.override_package_pin(pkg_name, pin_num, data)
    prev = engine.get_package_preview_data(pkg_name)
    return jsonify({
        "status": "ok" if success else "error",
        "preview": prev,
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/package/<pkg_name>/pin/<int:pin_num>/override", methods=["DELETE"])
def api_package_pin_reset(pkg_name: str, pin_num: int):
    success = engine.reset_package_pin(pkg_name, pin_num)
    prev = engine.get_package_preview_data(pkg_name)
    return jsonify({
        "status": "ok" if success else "error",
        "preview": prev,
        "stats": engine.get_panel_statistics()
    })


@app.route("/api/package/<pkg_name>/rename", methods=["POST"])
def api_package_rename(pkg_name: str):
    data = request.json or {}
    new_name = data.get("new_name", "").strip()
    if not new_name:
        return jsonify({"status": "error", "message": "Neuer Name darf nicht leer sein"}), 400
    success = engine.rename_package(pkg_name, new_name)
    return jsonify({
        "status": "ok" if success else "error",
        "old_name": pkg_name,
        "new_name": new_name,
        "packages": [p.to_dict() for p in engine.packages.values()]
    })


@app.route("/api/libraries", methods=["GET"])
def api_list_libraries():
    libs = engine.list_customer_libraries()
    return jsonify({
        "libraries": libs,
        "current_customer": engine.current_customer
    })


@app.route("/api/library/load", methods=["POST"])
def api_load_library():
    data = request.json or {}
    customer = data.get("customer", "Standard")
    success = engine.load_customer_library(customer)
    return jsonify({
        "status": "ok" if success else "error",
        "customer": engine.current_customer,
        "packages": [p.to_dict() for p in engine.packages.values()]
    })


@app.route("/api/library/save", methods=["POST"])
def api_save_library():
    data = request.json or {}
    customer = data.get("customer", engine.current_customer)
    path = engine.save_customer_library(customer)
    return jsonify({
        "status": "ok",
        "customer": engine.current_customer,
        "path": path
    })



if __name__ == "__main__":
    port = 5055
    print(f"==================================================")
    print(f" SMT Stencil Assistant laeuft auf:")
    print(f" http://127.0.0.1:{port}")
    print(f"==================================================")
    try:
        app.run(host="127.0.0.1", port=port, debug=False, threaded=True)
    except Exception as e:
        import traceback
        traceback.print_exc()

