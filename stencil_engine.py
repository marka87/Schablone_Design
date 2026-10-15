"""
SMT Stencil Design & Verification Engine
Hochperformante Geometrieverarbeitung, Multi-Layer Management, IPC-7525 Prüfung, Stufenzonen & Slicing.
"""

import math
import os
import re
import tempfile
import zipfile
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from typing import List, Dict, Tuple, Optional, Any

from gerbonara import GerberFile
from gerbonara.graphic_objects import Flash, Region
from gerbonara.apertures import RectangleAperture, CircleAperture
from gerbonara.utils import MM
from rtree import index


@dataclass
class ComponentItem:
    refdes: str
    package: str
    cx: float
    cy: float
    width: float
    height: float
    rotation: float = 0.0
    outline: List[Tuple[float, float]] = field(default_factory=list)
    side: str = "TOP"

    def to_dict(self) -> Dict[str, Any]:
        return {
            "refdes": self.refdes,
            "package": self.package,
            "cx": round(self.cx, 3),
            "cy": round(self.cy, 3),
            "width": round(self.width, 3),
            "height": round(self.height, 3),
            "rot": round(self.rotation, 1),
            "side": self.side,
            "outline": self.outline
        }


@dataclass
class StencilZone:
    id: str
    name: str
    x0: float
    y0: float
    x1: float
    y1: float
    thickness: float  # mm, z.B. 0.100 (Step-Down) oder 0.150 (Step-Up)
    is_step_down: bool = True

    @property
    def width(self) -> float:
        return abs(self.x1 - self.x0)

    @property
    def height(self) -> float:
        return abs(self.y1 - self.y0)

    def contains_point(self, x: float, y: float) -> bool:
        min_x, max_x = min(self.x0, self.x1), max(self.x0, self.x1)
        min_y, max_y = min(self.y0, self.y1), max(self.y0, self.y1)
        return (min_x <= x <= max_x) and (min_y <= y <= max_y)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "id": self.id,
            "name": self.name,
            "x0": round(min(self.x0, self.x1), 3),
            "y0": round(min(self.y0, self.y1), 3),
            "x1": round(max(self.x0, self.x1), 3),
            "y1": round(max(self.y0, self.y1), 3),
            "thickness": self.thickness,
            "is_step_down": self.is_step_down,
            "width": round(self.width, 3),
            "height": round(self.height, 3)
        }


@dataclass
class BoardLayer:
    id: str
    name: str
    filename: str
    layer_type: str  # 'paste', 'mask', 'copper', 'zone', 'profile', 'silk', 'component', 'fiducial', 'other'
    side: str = "top"  # 'top', 'bot', 'both'
    color: str = "#c084fc"
    visible: bool = True
    opacity: float = 0.85
    object_count: int = 0
    file_path: str = ""
    primitives: List[Dict[str, Any]] = field(default_factory=list)

    def to_dict(self, include_primitives: bool = False) -> Dict[str, Any]:
        d = {
            "id": self.id,
            "name": self.name,
            "filename": self.filename,
            "type": self.layer_type,
            "side": self.side,
            "color": self.color,
            "visible": self.visible,
            "opacity": self.opacity,
            "count": self.object_count,
            "file_path": self.file_path
        }
        if include_primitives:
            d["primitives"] = self.primitives
        return d


@dataclass
class PackageRule:
    name: str = "Simple Rectangle Surface"
    rule_type: str = "simple_rect"  # simple_rect, house_builder, inverted_house_builder, elongated_pitch, array_slicing, donut, take_existing, ignore
    undersize_w: float = 0.025      # mm (Reduktion in Breite)
    undersize_h: float = 0.025      # mm (Reduktion in Höhe)
    corner_radius: float = 0.075    # mm
    offset_x: float = 0.0           # mm
    offset_y: float = 0.0           # mm
    offset_move_away: float = 0.0   # mm
    house_cut_pct: float = 35.0     # %
    pitch_reduction_pct: float = 20.0 # %
    length_usage_pct: float = 95.0  # %
    bridge_count_x: int = 0
    bridge_count_y: int = 0
    bridge_width: float = 0.30      # mm
    donut_inner_dia: float = 0.0    # mm
    rotate_bridges_45: bool = False
    try_find_direction: bool = True

    def to_dict(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "rule_type": self.rule_type,
            "undersize_w": self.undersize_w,
            "undersize_h": self.undersize_h,
            "corner_radius": self.corner_radius,
            "offset_x": self.offset_x,
            "offset_y": self.offset_y,
            "offset_move_away": self.offset_move_away,
            "house_cut_pct": self.house_cut_pct,
            "pitch_reduction_pct": self.pitch_reduction_pct,
            "length_usage_pct": self.length_usage_pct,
            "bridge_count_x": self.bridge_count_x,
            "bridge_count_y": self.bridge_count_y,
            "bridge_width": self.bridge_width,
            "donut_inner_dia": self.donut_inner_dia,
            "rotate_bridges_45": self.rotate_bridges_45,
            "try_find_direction": self.try_find_direction
        }


@dataclass
class PackageTemplate:
    name: str
    pin_count: int
    pitch: float                # mm
    pad_width: float            # mm
    pad_height: float           # mm
    pads_relative: List[Dict[str, Any]] = field(default_factory=list)
    component_count: int = 1
    rule: PackageRule = field(default_factory=PackageRule)
    category: str = "SMD Passives"  # SMD Passives, ICs & Transistors, BGA & Arrays, Connectors, Other
    outline: Dict[str, float] = field(default_factory=lambda: {"w": 2.0, "h": 1.25})
    pad_overrides: Dict[int, Dict[str, Any]] = field(default_factory=dict)

    def to_dict(self) -> Dict[str, Any]:
        return {
            "name": self.name,
            "pin_count": self.pin_count,
            "pitch": round(self.pitch, 3),
            "pad_width": round(self.pad_width, 3),
            "pad_height": round(self.pad_height, 3),
            "component_count": self.component_count,
            "category": self.category,
            "outline": self.outline,
            "pads_relative": self.pads_relative,
            "rule": self.rule.to_dict(),
            "pad_overrides": self.pad_overrides
        }


@dataclass
class ApertureItem:
    id: int
    cx: float
    cy: float
    bbox: Tuple[float, float, float, float]  # min_x, min_y, max_x, max_y
    shape_type: str  # 'circle', 'rectangle', 'polygon'
    width: float
    height: float
    area: float
    perimeter: float
    points: List[Tuple[float, float]] = field(default_factory=list)  # Polygon vertices
    custom_slices: Optional[List[Dict[str, Any]]] = None  # Wenn geslict (2x2 etc.)
    original_obj: Any = None
    package_group: str = ""  # z.B. "0.21x0.21" oder Footprint-Typ
    package_name: str = ""   # z.B. "0805", "QFN48"
    cluster_id: int = -1
    component_id: int = -1
    pin_number: int = 1
    facing_dir: str = "right"  # "right", "left", "up", "down"
    is_manually_overridden: bool = False
    pad_ref: Optional[Dict[str, Any]] = None  # Zugehöriges Kupfer-Pad


class StencilEngine:
    def __init__(self, base_thickness: float = 0.120):
        self.base_thickness = base_thickness  # mm
        self.ipc_min_ar = 0.66   # IPC-7525 Area Ratio Minimum (Standard)
        self.ipc_min_aspect = 1.50 # IPC-7525 Aspect Ratio Minimum
        self.ipc_mode = "standard"  # 'standard', 'nano', 'custom'
        self.squeegee_dir = "Y"  # 'X' oder 'Y' (Druckrichtung)
        self.squeegee_clearance = 1.2  # mm Mindestabstand zur Stufenkante in Druckrichtung
        self.transverse_clearance = 0.8  # mm Querabstand

        self.apertures: List[ApertureItem] = []
        self.pads: List[Dict[str, Any]] = []
        self.components: List[ComponentItem] = []
        self.packages: Dict[str, PackageTemplate] = {}
        self.package_aliases: Dict[str, str] = {}
        self.current_customer: str = "Standard"
        self.zones: List[StencilZone] = []
        self.profile_paths: List[List[Tuple[float, float]]] = []
        self.fiducials: List[Tuple[float, float, float]] = []  # (x, y, r)

        # Multi-Layer Management
        self.layers: Dict[str, BoardLayer] = {}

        # Spatial Indexing
        self.spatial_index = index.Index()
        self.bounds = (0.0, 0.0, 0.0, 0.0)

        # Initialisiere Standard-Packages
        self.init_standard_packages()

    def set_ipc_mode(self, mode: str, custom_ar: float = 0.66, custom_aspect: float = 1.50):
        self.ipc_mode = mode
        if mode == "standard":
            self.ipc_min_ar = 0.66
            self.ipc_min_aspect = 1.50
        elif mode == "nano":
            self.ipc_min_ar = 0.55
            self.ipc_min_aspect = 1.30
        elif mode == "custom":
            self.ipc_min_ar = custom_ar
            self.ipc_min_aspect = custom_aspect

    def load_panel_dataset(self, 
                            paste_path: str, 
                            zone_path: Optional[str] = None, 
                            pad_path: Optional[str] = None,
                            profile_path: Optional[str] = None,
                            fiducial_path: Optional[str] = None):
        """Lade vollständigen Gerber-Datensatz und registriere Lagen im Layer-Manager."""
        self.apertures.clear()
        self.zones.clear()
        self.pads.clear()
        self.profile_paths.clear()
        self.fiducials.clear()
        self.layers.clear()
        self.spatial_index = index.Index()

        # Ermittle Basisdicke aus Dateinamen, z.B. _120 -> 0.120 mm
        m = re.search(r'_(\d{2,3})\.ger$', paste_path, re.IGNORECASE)
        if m:
            self.base_thickness = int(m.group(1)) / 1000.0

        # 1. Zonen laden (falls vorhanden)
        if zone_path and os.path.exists(zone_path):
            self._load_zones_gerber(zone_path)
            self.add_layer_from_gerber(zone_path, layer_type="zone", color="#f59e0b")

        # 2. Aperturen (Paste Layer) laden
        if paste_path and os.path.exists(paste_path):
            self._load_paste_gerber(paste_path)
            self.add_layer_from_gerber(paste_path, layer_type="paste", color="#c084fc")

        # 3. Pads (Kupfer Layer) laden (falls vorhanden)
        if pad_path and os.path.exists(pad_path):
            self._load_pads_gerber(pad_path)
            self.add_layer_from_gerber(pad_path, layer_type="mask" if "sm" in pad_path.lower() else "copper", color="#06b6d4" if "sm" in pad_path.lower() else "#10b981")

        # 4. Profil / Kontur laden
        if profile_path and os.path.exists(profile_path):
            self._load_profile_gerber(profile_path)
            self.add_layer_from_gerber(profile_path, layer_type="profile", color="#94a3b8")

        # 5. Fiducials laden
        if fiducial_path and os.path.exists(fiducial_path):
            self._load_fiducials_gerber(fiducial_path)
            self.add_layer_from_gerber(fiducial_path, layer_type="fiducial", color="#f43f5e")

        # 6. Korreliere Aperturen mit Pads (falls Pads geladen)
        self._correlate_apertures_with_pads()

        # 7. Automatische Package-Erkennung & Gruppierung
        self.detect_packages()
        self.load_customer_library(self.current_customer)

    # -------------------------------------------------------------
    # Multi-Layer Management & Windows Import
    # -------------------------------------------------------------
    def parse_gerber_to_primitives(self, file_path: str) -> List[Dict[str, Any]]:
        """Liest Vektorgeometrie aus einer beliebigen Gerberdatei für das Canvas aus."""
        prims = []
        try:
            g = GerberFile.open(file_path)
            for obj in g.objects:
                for p in obj.to_primitives(unit=MM):
                    pname = type(p).__name__
                    if pname == 'Rectangle':
                        prims.append({"t": "r", "x": round(p.x, 3), "y": round(p.y, 3), "w": round(p.w, 3), "h": round(p.h, 3)})
                    elif pname == 'Circle':
                        d = p.diameter if hasattr(p, 'diameter') else p.r * 2
                        prims.append({"t": "c", "x": round(p.x, 3), "y": round(p.y, 3), "r": round(d/2, 3)})
                    elif pname == 'Line':
                        prims.append({"t": "l", "x1": round(p.x1, 3), "y1": round(p.y1, 3), "x2": round(p.x2, 3), "y2": round(p.y2, 3), "w": round(p.width, 3)})
                    elif pname == 'ArcPoly':
                        pts = [[round(pt[0], 3), round(pt[1], 3)] for pt in p.outline]
                        if len(pts) >= 3:
                            prims.append({"t": "p", "pts": pts})
        except Exception as e:
            print(f"Warnung beim Parsen von {file_path}: {e}")
        return prims

    def add_layer_from_gerber(self, 
                              file_path: str, 
                              layer_name: Optional[str] = None, 
                              layer_type: Optional[str] = None, 
                              color: Optional[str] = None, 
                              side: Optional[str] = None) -> BoardLayer:
        """Erstellt eine neue Lage aus einer beliebigen Gerber-Datei."""
        bn = os.path.basename(file_path)
        bn_lower = bn.lower()

        if not side:
            side = "bot" if ("bot" in bn_lower or "bottom" in bn_lower or "smb" in bn_lower) else "top"

        if not layer_type:
            if "zone" in bn_lower or "step" in bn_lower:
                layer_type = "zone"
                color = color or "#f59e0b"  # Orange
            elif "panel" in bn_lower or "paste" in bn_lower or "spt" in bn_lower or "spb" in bn_lower:
                layer_type = "paste"
                color = color or "#c084fc"  # Pink / Lavender
            elif "profile" in bn_lower or "outline" in bn_lower:
                layer_type = "profile"
                color = color or "#94a3b8"  # Grau / Weiß
            elif "fiducial" in bn_lower or "passer" in bn_lower:
                layer_type = "fiducial"
                color = color or "#f43f5e"  # Rose
            elif "smt" in bn_lower or "mask" in bn_lower or "lötstopp" in bn_lower or "smb" in bn_lower:
                layer_type = "mask"
                color = color or "#06b6d4"  # Cyan
            elif "copper" in bn_lower or "top" in bn_lower or "bot" in bn_lower or "signal" in bn_lower or "cu" in bn_lower:
                layer_type = "copper"
                color = color or "#10b981"  # Grün
            elif "comp" in bn_lower or "silk" in bn_lower or "bestückung" in bn_lower:
                layer_type = "component"
                color = color or "#facc15"  # Gelb
            else:
                layer_type = "other"
                color = color or "#a855f7"

        clean_id = f"layer_{re.sub(r'[^a-zA-Z0-9_]', '_', bn)}"
        prims = self.parse_gerber_to_primitives(file_path)

        layer = BoardLayer(
            id=clean_id,
            name=layer_name or bn,
            filename=bn,
            layer_type=layer_type,
            side=side,
            color=color or "#c084fc",
            visible=True,
            opacity=0.85,
            object_count=len(prims),
            file_path=file_path,
            primitives=prims
        )
        self.layers[clean_id] = layer
        return layer

    def import_files_or_zip(self, file_paths: List[str], side: str = "top", clear_previous: bool = False) -> List[BoardLayer]:
        """
        Importiert mehrere Gerber-Dateien, ein ODB++ ZIP-Archiv oder ein ODB++ Verzeichnis.
        Erzeugt automatisch sichtbare Layer im Layer-Manager mit Checkboxen (Haken).
        """
        if clear_previous:
            self.close_dataset()

        imported_layers = []
        for fp in file_paths:
            if not os.path.exists(fp):
                continue

            # 1. Direktes Verzeichnis (z.B. entpacktes ODB++ Verzeichnis)
            if os.path.isdir(fp):
                matrix_file = None
                for root, dirs, files in os.walk(fp):
                    if "matrix" in files and os.path.basename(root) == "matrix":
                        matrix_file = os.path.join(root, "matrix")
                        extract_dir = os.path.dirname(root)
                        break

                if matrix_file:
                    odb_layers = self._import_odb_matrix_layers(extract_dir, matrix_file, side=side)
                    imported_layers.extend(odb_layers)
                else:
                    dir_layers = self.import_directory(fp, side=side)
                    imported_layers.extend(dir_layers)
                continue

            # 2. ZIP-Archiv (ODB++ oder Gerber)
            if fp.lower().endswith(('.zip', '.tgz', '.tar.gz')):
                extract_dir = os.path.join(tempfile.gettempdir(), f"stencil_import_{os.path.basename(fp)}")
                os.makedirs(extract_dir, exist_ok=True)
                with zipfile.ZipFile(fp, 'r') as z:
                    z.extractall(extract_dir)

                # Prüfe ODB++ matrix
                matrix_file = None
                for root, dirs, files in os.walk(extract_dir):
                    if "matrix" in files and os.path.basename(root) == "matrix":
                        matrix_file = os.path.join(root, "matrix")
                        # extract_dir ist das Basisverzeichnis über matrix
                        extract_dir = os.path.dirname(root)
                        break

                if matrix_file:
                    odb_layers = self._import_odb_matrix_layers(extract_dir, matrix_file, side=side)
                    imported_layers.extend(odb_layers)
                else:
                    # Gerber ZIP Dateien einlesen
                    paste_candidates = []
                    for root, dirs, files in os.walk(extract_dir):
                        for f in files:
                            fl = f.lower()
                            if fl.endswith(('.ger', '.gbr', '.pho')):
                                full_p = os.path.join(root, f)
                                layer = self.add_layer_from_gerber(full_p)
                                imported_layers.append(layer)
                                if layer.layer_type == "paste" and side in layer.side:
                                    paste_candidates.append(full_p)

                    if paste_candidates:
                        self._load_paste_gerber(paste_candidates[0])

                    zone_candidates = [l.file_path for l in imported_layers if l.layer_type == "zone" and side in l.side]
                    if zone_candidates:
                        self._load_zones_gerber(zone_candidates[0])

                    profile_candidates = [l.file_path for l in imported_layers if l.layer_type == "profile" and side in l.side]
                    if profile_candidates:
                        self._load_profile_gerber(profile_candidates[0])

                    fid_candidates = [l.file_path for l in imported_layers if l.layer_type == "fiducial" and side in l.side]
                    if fid_candidates:
                        self._load_fiducials_gerber(fid_candidates[0])

                    pad_candidates = [l.file_path for l in imported_layers if l.layer_type in ("mask", "copper") and side in l.side]
                    if pad_candidates:
                        self._load_pads_gerber(pad_candidates[0])
                        self._correlate_apertures_with_pads()

                    if paste_candidates:
                        self.detect_packages()
            else:
                # Einzelne Gerber-Datei importieren
                layer = self.add_layer_from_gerber(fp)
                imported_layers.append(layer)
                if layer.layer_type == "paste":
                    self._load_paste_gerber(fp)
                    self.detect_packages()
                elif layer.layer_type == "zone":
                    self._load_zones_gerber(fp)
                elif layer.layer_type in ("mask", "copper"):
                    self._load_pads_gerber(fp)
                    self._correlate_apertures_with_pads()
                elif layer.layer_type == "profile":
                    self._load_profile_gerber(fp)
                elif layer.layer_type == "fiducial":
                    self._load_fiducials_gerber(fp)

        return imported_layers

    def _parse_odb_symbol(self, sym_str: str, is_inch: bool = True) -> Dict[str, Any]:
        """Konvertiert ODB++ Symbol-Definition (Mils in Inches-Modus) in mm."""
        unit_m = 0.0254 if is_inch else 1.0
        m = re.match(r'rect([\d\.]+)x([\d\.]+)(?:xr([\d\.]+))?', sym_str)
        if m:
            w = float(m.group(1)) * unit_m
            h = float(m.group(2)) * unit_m
            r = float(m.group(3)) * unit_m if m.group(3) else 0.0
            return {'type': 'r', 'w': round(w, 4), 'h': round(h, 4), 'r': round(r, 4)}

        m = re.match(r'r([\d\.]+)', sym_str)
        if m:
            dia = float(m.group(1)) * unit_m
            return {'type': 'c', 'r': round(dia / 2.0, 4), 'w': round(dia, 4), 'h': round(dia, 4)}

        m = re.match(r's([\d\.]+)', sym_str)
        if m:
            side = float(m.group(1)) * unit_m
            return {'type': 'r', 'w': round(side, 4), 'h': round(side, 4), 'r': 0.0}

        m = re.match(r'oval([\d\.]+)x([\d\.]+)', sym_str)
        if m:
            w = float(m.group(1)) * unit_m
            h = float(m.group(2)) * unit_m
            return {'type': 'oval', 'w': round(w, 4), 'h': round(h, 4)}

        return {'type': 'r', 'w': 0.5, 'h': 0.5, 'r': 0.0}

    def _parse_odb_features(self, feat_file: str, max_prims: int = 20000) -> Tuple[List[Dict[str, Any]], Tuple[float, float, float, float]]:
        """Liest ODB++ features Datei ein (Pads, Lines, Arcs, Surfaces) und liefert Vektoren + Bounding-Box."""
        prims = []
        min_x, min_y, max_x, max_y = 999999.0, 999999.0, -999999.0, -999999.0
        symbols = {}
        coord_scale = 25.4  # Default INCH
        is_inch = True

        if not os.path.exists(feat_file) or os.path.getsize(feat_file) == 0:
            return prims, (0.0, 0.0, 0.0, 0.0)

        try:
            with open(feat_file, "r", encoding="utf-8", errors="ignore") as f:
                lines = f.readlines()

            in_surface = False
            cur_pts = []

            for line in lines:
                line = line.strip()
                if not line or line.startswith(('#', '@', '&')):
                    continue

                if line.startswith('UNITS='):
                    unit = line.split('=')[1].strip()
                    is_inch = (unit == 'INCH')
                    coord_scale = 25.4 if is_inch else 1.0
                    continue

                if line.startswith('$'):
                    parts = line[1:].split()
                    sid = parts[0]
                    sdef = parts[1] if len(parts) > 1 else ''
                    symbols[sid] = self._parse_odb_symbol(sdef, is_inch=is_inch)
                    continue

                # Pad / Flash
                if line.startswith('P '):
                    parts = line.split()
                    if len(parts) >= 4:
                        try:
                            cx = float(parts[1]) * coord_scale
                            cy = float(parts[2]) * coord_scale
                            sid = parts[3]
                            rot = float(parts[5]) if len(parts) > 5 and parts[5].replace('.', '', 1).isdigit() else 0.0
                            sym = symbols.get(sid, {'type': 'r', 'w': 0.5, 'h': 0.5, 'r': 0.0})

                            pw, ph = sym.get('w', 0.5), sym.get('h', 0.5)
                            min_x = min(min_x, cx - pw/2)
                            max_x = max(max_x, cx + pw/2)
                            min_y = min(min_y, cy - ph/2)
                            max_y = max(max_y, cy + ph/2)

                            if len(prims) < max_prims:
                                if sym['type'] == 'c':
                                    prims.append({"t": "c", "x": round(cx, 3), "y": round(cy, 3), "r": sym['r']})
                                else:
                                    prims.append({
                                        "t": "r", "x": round(cx, 3), "y": round(cy, 3),
                                        "w": pw, "h": ph, "rot": rot, "r": sym.get('r', 0.0)
                                    })
                        except Exception:
                            pass
                    continue

                # Line
                if line.startswith('L '):
                    parts = line.split()
                    if len(parts) >= 5:
                        try:
                            x0 = float(parts[1]) * coord_scale
                            y0 = float(parts[2]) * coord_scale
                            x1 = float(parts[3]) * coord_scale
                            y1 = float(parts[4]) * coord_scale
                            sid = parts[5] if len(parts) > 5 else '0'
                            sym = symbols.get(sid, {'w': 0.15})
                            lw = sym.get('w', 0.15)

                            min_x = min(min_x, x0, x1)
                            max_x = max(max_x, x0, x1)
                            min_y = min(min_y, y0, y1)
                            max_y = max(max_y, y0, y1)

                            if len(prims) < max_prims:
                                prims.append({
                                    "t": "l",
                                    "x0": round(x0, 3), "y0": round(y0, 3),
                                    "x1": round(x1, 3), "y1": round(y1, 3),
                                    "x2": round(x1, 3), "y2": round(y1, 3),
                                    "w": round(lw, 3)
                                })
                        except Exception:
                            pass
                    continue

                # Arc
                if line.startswith('A '):
                    parts = line.split()
                    if len(parts) >= 7:
                        try:
                            x0 = float(parts[1]) * coord_scale
                            y0 = float(parts[2]) * coord_scale
                            x1 = float(parts[3]) * coord_scale
                            y1 = float(parts[4]) * coord_scale
                            xc = float(parts[5]) * coord_scale
                            yc = float(parts[6]) * coord_scale
                            sid = parts[7] if len(parts) > 7 else '0'
                            sym = symbols.get(sid, {'w': 0.15})

                            min_x = min(min_x, x0, x1)
                            max_x = max(max_x, x0, x1)
                            min_y = min(min_y, y0, y1)
                            max_y = max(max_y, y0, y1)

                            if len(prims) < max_prims:
                                prims.append({
                                    "t": "l",
                                    "x0": round(x0, 3), "y0": round(y0, 3),
                                    "x1": round(x1, 3), "y1": round(y1, 3),
                                    "x2": round(x1, 3), "y2": round(y1, 3),
                                    "w": round(sym.get('w', 0.15), 3)
                                })
                        except Exception:
                            pass
                    continue

                # Surfaces
                if line.startswith('S '):
                    in_surface = True
                    cur_pts = []
                    continue

                if in_surface:
                    if line.startswith(('OB ', 'OS ')):
                        parts = line.split()
                        if len(parts) >= 3:
                            px = float(parts[1]) * coord_scale
                            py = float(parts[2]) * coord_scale
                            cur_pts.append((round(px, 3), round(py, 3)))
                            min_x = min(min_x, px)
                            max_x = max(max_x, px)
                            min_y = min(min_y, py)
                            max_y = max(max_y, py)
                    elif line.startswith('OE'):
                        if cur_pts and len(cur_pts) >= 3 and len(prims) < max_prims:
                            prims.append({"t": "p", "pts": cur_pts})
                        cur_pts = []
                    elif line.startswith('SE'):
                        in_surface = False
                        cur_pts = []

            if min_x > max_x:
                bbox = (0.0, 0.0, 0.0, 0.0)
            else:
                bbox = (round(min_x, 2), round(min_y, 2), round(max_x, 2), round(max_y, 2))
            return prims, bbox
        except Exception as e:
            print(f"Fehler beim Parsen von ODB Features {feat_file}: {e}")
            return prims, (0.0, 0.0, 0.0, 0.0)

    def _categorize_package_name(self, name: str) -> str:
        """Kategorisiert Bauteil-Footprints für den Stencil Generator."""
        nl = name.lower()
        if any(x in nl for x in ('0201', '0402', '0603', '0805', '1206', '1210', 'cap', 'res', 'dio', 'melf', 'ko', 'wi')):
            return "SMD Passives"
        elif any(x in nl for x in ('qfn', 'qfp', 'soic', 'sop', 'sot', 'tssop', 'dfn', 'ic', 'to-2')):
            return "ICs & Transistors"
        elif any(x in nl for x in ('bga', 'lga', 'csp', 'array')):
            return "BGA & Arrays"
        elif any(x in nl for x in ('harting', 'phoenix', 'te_', 'conn', 'molex', 'relais', 'cvilux')):
            return "Connectors"
        else:
            return "Other"

    def _parse_odb_eda_data(self, eda_file: str) -> List[Dict[str, Any]]:
        """Liest Bauteil-Packages und Pins aus steps/<step>/eda/data ein."""
        packages_list = []
        if not os.path.exists(eda_file):
            return packages_list

        scale = 25.4
        current_pkg = None

        try:
            with open(eda_file, "r", encoding="utf-8", errors="ignore") as f:
                for line in f:
                    line = line.strip()
                    if not line or line.startswith('#'):
                        continue

                    if line.startswith('UNITS='):
                        scale = 25.4 if 'INCH' in line else 1.0
                        continue

                    if line.startswith('PKG '):
                        parts = line.split()
                        if len(parts) >= 7:
                            pkg_name = parts[1]
                            pitch = float(parts[2]) * scale
                            xmin = float(parts[3]) * scale
                            ymin = float(parts[4]) * scale
                            xmax = float(parts[5]) * scale
                            ymax = float(parts[6]) * scale
                            pw = max(0.5, xmax - xmin)
                            ph = max(0.5, ymax - ymin)

                            current_pkg = {
                                "name": pkg_name,
                                "pitch": round(pitch, 3),
                                "w": round(pw, 3),
                                "h": round(ph, 3),
                                "pins": []
                            }
                            packages_list.append(current_pkg)
                        continue

                    if line.startswith('PIN ') and current_pkg is not None:
                        parts = line.split()
                        if len(parts) >= 5:
                            try:
                                pin_idx = int(parts[1]) if parts[1].isdigit() else len(current_pkg["pins"]) + 1
                                pin_x = float(parts[3]) * scale
                                pin_y = float(parts[4]) * scale
                                current_pkg["pins"].append({
                                    "pin": pin_idx,
                                    "cx": round(pin_x, 3),
                                    "cy": round(pin_y, 3),
                                    "w": round(min(current_pkg["w"] / 2.5, 0.8), 3),
                                    "h": round(min(current_pkg["h"] / 2.5, 0.8), 3),
                                    "facing": "right" if pin_x > 0 else "left"
                                })
                            except Exception:
                                pass

            # In self.packages als Templates registrieren
            for p in packages_list:
                name = p["name"]
                pins = p["pins"]
                pin_cnt = len(pins) if pins else 2
                pitch = p["pitch"] if p["pitch"] > 0 else 1.0

                rule = PackageRule(name="Simple Rectangle Surface", undersize_w=0.025, undersize_h=0.025, corner_radius=0.075)
                if "qfn" in name.lower():
                    rule = PackageRule(name="HouseBuilder", rule_type="house_builder", undersize_w=0.025, undersize_h=0.025, house_cut_pct=35.0)

                self.packages[name] = PackageTemplate(
                    name=name,
                    pin_count=pin_cnt,
                    pitch=pitch,
                    pad_width=round(p["w"] / 3.0, 2),
                    pad_height=round(p["h"] / 3.0, 2),
                    category=self._categorize_package_name(name),
                    outline={"w": p["w"], "h": p["h"]},
                    pads_relative=pins if pins else [
                        {"pin": 1, "cx": round(-pitch/2, 3), "cy": 0.0, "w": 0.5, "h": 0.8, "facing": "right"},
                        {"pin": 2, "cx": round(pitch/2, 3), "cy": 0.0, "w": 0.5, "h": 0.8, "facing": "left"}
                    ],
                    rule=rule
                )

        except Exception as e:
            print(f"Fehler beim Lesen von ODB EDA Data: {e}")

        return packages_list

    def _parse_odb_components_file(self, comp_file: str, side: str, packages_list: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
        """Liest ODB++ components Datei aus und erzeugt ComponentItem + Visual Primitives."""
        comp_prims = []
        if not os.path.exists(comp_file):
            return comp_prims

        scale = 25.4  # Default INCH
        try:
            with open(comp_file, "r", encoding="utf-8", errors="ignore") as f:
                for line in f:
                    line = line.strip()
                    if not line or line.startswith('#'):
                        continue

                    if line.startswith('UNITS='):
                        scale = 25.4 if 'INCH' in line else 1.0
                        continue

                    if line.startswith('CMP '):
                        parts = line.split()
                        if len(parts) >= 6:
                            try:
                                pkg_id = int(parts[1]) if parts[1].isdigit() else -1
                                cx = float(parts[2]) * scale
                                cy = float(parts[3]) * scale
                                rot = float(parts[4]) if parts[4].replace('.', '', 1).isdigit() else 0.0
                                refdes = parts[6] if len(parts) > 6 else parts[5]
                                if ';' in refdes:
                                    refdes = refdes.split(';')[0]

                                pkg_name = "GENERIC"
                                pw, ph = 1.6, 1.0
                                if 0 <= pkg_id < len(packages_list):
                                    pkg_info = packages_list[pkg_id]
                                    pkg_name = pkg_info["name"]
                                    pw = pkg_info["w"]
                                    ph = pkg_info["h"]

                                # Bauteil in self.components registrieren
                                self.components.append(ComponentItem(
                                    refdes=refdes,
                                    package=pkg_name,
                                    cx=round(cx, 3),
                                    cy=round(cy, 3),
                                    width=round(pw, 3),
                                    height=round(ph, 3),
                                    rotation=rot,
                                    side=side
                                ))

                                # Primitives für Layer
                                comp_prims.append({
                                    "t": "comp",
                                    "refdes": refdes,
                                    "pkg": pkg_name,
                                    "x": round(cx, 3),
                                    "y": round(cy, 3),
                                    "w": round(pw, 3),
                                    "h": round(ph, 3),
                                    "rot": rot
                                })
                            except Exception:
                                pass
        except Exception as e:
            print(f"Fehler beim Parsen von ODB Komponenten {comp_file}: {e}")

        return comp_prims

    def _import_odb_matrix_layers(self, extract_dir: str, matrix_file: str, side: str = "top") -> List[BoardLayer]:
        """Liest vollständige ODB++ Lagen, EDA Packages, Bauteile und Vektoren ein."""
        layers = []
        try:
            # 1. Aktiven Board-Step ermitteln (Nutzen weglassen!)
            steps_dir = os.path.join(extract_dir, "steps")
            board_step = None
            if os.path.exists(steps_dir):
                all_steps = os.listdir(steps_dir)
                # Filter: panel, nutz, template ausschließen wenn echter Board-Step da
                board_candidates = [s for s in all_steps if not any(x in s.lower() for x in ('panel', 'nutz', 'template'))]
                if board_candidates:
                    board_step = board_candidates[0]
                elif all_steps:
                    board_step = all_steps[0]

            step_dir = os.path.join(steps_dir, board_step) if board_step else extract_dir
            layers_base = os.path.join(step_dir, "layers")
            eda_file = os.path.join(step_dir, "eda", "data")

            # 2. EDA Packages einlesen
            eda_packages = self._parse_odb_eda_data(eda_file)

            # 3. Matrix einlesen
            with open(matrix_file, "r", encoding="utf-8", errors="ignore") as f:
                content = f.read()

            raw_layers = re.findall(r'LAYER\s*\{[^}]*TYPE=([^\s]+)[^}]*NAME=([^\s]+)', content)
            if not raw_layers:
                # Fallback: Simple Regex
                layer_names = re.findall(r'NAME\s*=\s*([a-zA-Z0-9_\+\-]+)', content)
                layer_types = re.findall(r'TYPE\s*=\s*([a-zA-Z0-9_\+\-]+)', content)
                raw_layers = list(zip(layer_types, layer_names))

            overall_min_x, overall_min_y, overall_max_x, overall_max_y = 999999.0, 999999.0, -999999.0, -999999.0

            # 4. Lagen parsen und Vektoren erzeugen
            for ltype_raw, name_raw in raw_layers:
                name = name_raw.strip().lower()
                clean_id = f"odb_{name}"
                lt = ltype_raw.upper()

                if "PASTE" in lt or name in ("spt", "spb", "top_paste", "bottom_paste"):
                    ltype = "paste"
                    color = "#c084fc"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"
                elif "MASK" in lt or name in ("smt", "smb", "top_solder", "bottom_solder"):
                    ltype = "mask"
                    color = "#06b6d4"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"
                elif "SIGNAL" in lt or "POWER" in lt or name in ("top", "bot", "top_layer", "bottom_layer"):
                    ltype = "copper"
                    color = "#10b981"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"
                elif "COMPONENT" in lt or "comp" in name:
                    ltype = "component"
                    color = "#facc15"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"
                elif "ROUT" in lt or name in ("outline", "profile"):
                    ltype = "profile"
                    color = "#94a3b8"
                    lside = "both"
                elif "DRILL" in lt:
                    ltype = "drill"
                    color = "#ef4444"
                    lside = "both"
                elif "SILK" in lt:
                    ltype = "silk"
                    color = "#f1f5f9"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"
                else:
                    ltype = "other"
                    color = "#a855f7"
                    lside = "bot" if ("bot" in name or "b" in name) else "top"

                # Layer Primitives initialisieren
                prims = []
                bbox = (0.0, 0.0, 0.0, 0.0)

                # Komponenten-Lage aus components-Datei lesen
                if ltype == "component" or "comp" in name:
                    comp_file = os.path.join(layers_base, name, "components")
                    if os.path.exists(comp_file):
                        prims = self._parse_odb_components_file(comp_file, side=lside.upper(), packages_list=eda_packages)

                # Features-Datei lesen (falls vorhanden)
                feat_file = os.path.join(layers_base, name, "features")
                if os.path.exists(feat_file):
                    feat_prims, feat_bbox = self._parse_odb_features(feat_file)
                    if not prims:
                        prims = feat_prims
                    else:
                        prims.extend(feat_prims[:5000])
                    bbox = feat_bbox
                    if bbox[0] < bbox[2]:
                        overall_min_x = min(overall_min_x, bbox[0])
                        overall_min_y = min(overall_min_y, bbox[1])
                        overall_max_x = max(overall_max_x, bbox[2])
                        overall_max_y = max(overall_max_y, bbox[3])

                # Standard-Sichtbarkeit: Nur relevante Aussenlagen sofort sichtbar (Kopien und Innenlagen standardmäßig aus)
                is_active_paste = (ltype == "paste" and side in lside and "copy" not in name)
                is_active_comp = (ltype == "component" and side in lside and "copy" not in name)
                is_active_solder = (ltype == "mask" and side in lside and "copy" not in name)
                is_active_copper = (ltype == "copper" and side in lside and "signal" not in name and "copy" not in name)
                is_outline = (ltype == "profile" and "copy" not in name)

                is_visible = is_active_paste or is_active_comp or is_active_solder or is_active_copper or is_outline

                layer = BoardLayer(
                    id=clean_id,
                    name=name_raw,
                    filename=name,
                    layer_type=ltype,
                    side=lside,
                    color=color,
                    visible=is_visible,
                    opacity=0.85,
                    object_count=len(prims),
                    primitives=prims
                )
                self.layers[clean_id] = layer
                layers.append(layer)

            # 5. Aktive Pasten-Lage als Aperturen für das Schablonendesign laden
            active_paste_name = f"top_paste" if side == "top" else f"bottom_paste"
            paste_feat_file = os.path.join(layers_base, active_paste_name, "features")
            if not os.path.exists(paste_feat_file):
                # Fallback Suche nach einer Paste-Lage
                for l in layers:
                    if l.layer_type == "paste" and side in l.side:
                        cand = os.path.join(layers_base, l.filename, "features")
                        if os.path.exists(cand):
                            paste_feat_file = cand
                            break

            if os.path.exists(paste_feat_file):
                self._load_odb_paste_apertures(paste_feat_file)

            if overall_min_x < overall_max_x:
                self.bounds = (overall_min_x, overall_min_y, overall_max_x, overall_max_y)

        except Exception as e:
            import traceback
            traceback.print_exc()
            print(f"ODB Matrix Fehler: {e}")

        return layers

    def _load_odb_paste_apertures(self, feat_file: str):
        """Erzeugt interaktive ApertureItem Objekte aus einer ODB++ Paste-Features Datei."""
        self.apertures.clear()
        self.spatial_index = index.Index()
        symbols = {}
        coord_scale = 25.4
        is_inch = True

        try:
            with open(feat_file, "r", encoding="utf-8", errors="ignore") as f:
                lines = f.readlines()

            for line in lines:
                line = line.strip()
                if not line or line.startswith(('#', '@', '&')):
                    continue

                if line.startswith('UNITS='):
                    unit = line.split('=')[1].strip()
                    is_inch = (unit == 'INCH')
                    coord_scale = 25.4 if is_inch else 1.0
                    continue

                if line.startswith('$'):
                    parts = line[1:].split()
                    sid = parts[0]
                    sdef = parts[1] if len(parts) > 1 else ''
                    symbols[sid] = self._parse_odb_symbol(sdef, is_inch=is_inch)
                    continue

                if line.startswith('P '):
                    parts = line.split()
                    if len(parts) >= 4:
                        cx = float(parts[1]) * coord_scale
                        cy = float(parts[2]) * coord_scale
                        sid = parts[3]
                        sym = symbols.get(sid, {'type': 'r', 'w': 0.8, 'h': 0.8})

                        pw = sym.get('w', 0.8)
                        ph = sym.get('h', 0.8)
                        shape_type = 'circle' if sym.get('type') == 'c' else 'rectangle'

                        ap_id = len(self.apertures)
                        bbox = (cx - pw/2, cy - ph/2, cx + pw/2, cy + ph/2)
                        area = math.pi * ((pw/2)**2) if shape_type == 'circle' else pw * ph
                        perimeter = math.pi * pw if shape_type == 'circle' else 2 * (pw + ph)

                        # Nahegelegenes Bauteil suchen
                        comp_match = None
                        min_d = 4.0
                        for c in self.components:
                            d = math.hypot(c.cx - cx, c.cy - cy)
                            if d < min_d:
                                min_d = d
                                comp_match = c

                        pkg_name = comp_match.package if comp_match else f"{round(pw, 2)}x{round(ph, 2)}"
                        refdes_id = comp_match.refdes if comp_match else ""

                        item = ApertureItem(
                            id=ap_id,
                            cx=cx,
                            cy=cy,
                            bbox=bbox,
                            shape_type=shape_type,
                            width=pw,
                            height=ph,
                            area=area,
                            perimeter=perimeter,
                            package_group=f"{round(pw, 2):.2f}x{round(ph, 2):.2f}",
                            package_name=pkg_name,
                            component_id=refdes_id
                        )
                        self.apertures.append(item)
                        self.spatial_index.insert(ap_id, bbox)

            print(f"Erfolgreich {len(self.apertures)} Schablonen-Pads aus ODB++ geladen.")
            if self.apertures:
                min_x = min(a.cx - a.width/2 for a in self.apertures)
                max_x = max(a.cx + a.width/2 for a in self.apertures)
                min_y = min(a.cy - a.height/2 for a in self.apertures)
                max_y = max(a.cy + a.height/2 for a in self.apertures)
                self.bounds = (min_x - 5, min_y - 5, max_x + 5, max_y + 5)
        except Exception as e:
            print(f"Fehler beim Erzeugen der ODB Aperturen: {e}")

    def get_layers_summary(self) -> List[Dict[str, Any]]:
        """Gibt Metadaten aller geladenen Lagen für die UI-Tabelle zurück."""
        res = []
        for l in self.layers.values():
            res.append(l.to_dict(include_primitives=False))
        return res

    def get_layer_primitives_map(self) -> Dict[str, Any]:
        """Gibt Vektordaten aller sichtbaren Lagen für das Canvas-Rendering zurück."""
        data = {}
        for lid, l in self.layers.items():
            if l.visible:
                data[lid] = {
                    "color": l.color,
                    "opacity": l.opacity,
                    "type": l.layer_type,
                    "prims": l.primitives
                }
        return data

    def update_layer_settings(self, 
                              layer_id: str, 
                              visible: Optional[bool] = None, 
                              color: Optional[str] = None, 
                              opacity: Optional[float] = None,
                              layer_type: Optional[str] = None):
        """Aktualisiert Sichtbarkeit (Haken), Farbe und Deckkraft einer Lage."""
        if layer_id in self.layers:
            l = self.layers[layer_id]
            if visible is not None:
                l.visible = bool(visible)
            if color:
                l.color = color
            if opacity is not None:
                l.opacity = float(opacity)
            if layer_type:
                l.layer_type = layer_type

    def remove_layer(self, layer_id: str):
        """Entfernt eine Lage aus dem Layer-Manager."""
        if layer_id in self.layers:
            del self.layers[layer_id]

    def load_zip_archive(self, zip_path: str, side: str = "top") -> List[BoardLayer]:
        """Lädt entweder ein ODB++ ZIP oder ein Gerber-ZIP sauber ein."""
        return self.import_files_or_zip([zip_path], side=side, clear_previous=True)

    def import_directory(self, dir_path: str, side: str = "top") -> List[BoardLayer]:
        """Importiert alle Gerber/ODB-Dateien aus einem gewählten Windows-Ordner."""
        return self.import_files_or_zip([dir_path], side=side, clear_previous=True)

    def _correlate_apertures_with_pads(self):
        """Findet zu jeder Pasten-Apertur das korrespondierende Kupfer-Pad."""
        if not self.pads or not self.apertures:
            return

        for ap in self.apertures:
            best_pad = None
            min_dist = 0.5  # max 0.5 mm Mittenversatz
            for p in self.pads:
                pcx = (p["x0"] + p["x1"]) / 2.0
                pcy = (p["y0"] + p["y1"]) / 2.0
                dist = math.hypot(ap.cx - pcx, ap.cy - pcy)
                if dist < min_dist:
                    min_dist = dist
                    best_pad = p

            if best_pad:
                ap.pad_ref = best_pad

    def _load_zones_gerber(self, path: str):
        """Liest Stufenzonen aus Gerber-Datei ein."""
        try:
            with open(path, "r", encoding="utf-8", errors="ignore") as f:
                lines = f.readlines()

            cur_poly = []
            polys = []
            for l in lines:
                m = re.match(r'(?:G0[123])?X(-?\d+)Y(-?\d+)(?:I-?\d+)?(?:J-?\d+)?D0([12])\*', l.strip())
                if m:
                    x, y, d = int(m.group(1))/1e5, int(m.group(2))/1e5, m.group(3)
                    if d == '2':
                        if cur_poly:
                            polys.append(cur_poly)
                        cur_poly = [(x, y)]
                    else:
                        cur_poly.append((x, y))
            if cur_poly:
                polys.append(cur_poly)

            zone_idx = 1
            for p in polys:
                min_x = min(pt[0] for pt in p)
                max_x = max(pt[0] for pt in p)
                min_y = min(pt[1] for pt in p)
                max_y = max(pt[1] for pt in p)
                w, h = max_x - min_x, max_y - min_y
                if w >= 2.5 and h >= 2.5:
                    z = StencilZone(
                        id=f"zone_{zone_idx}",
                        name=f"Zone {zone_idx} (Step-Down)",
                        x0=min_x,
                        y0=min_y,
                        x1=max_x,
                        y1=max_y,
                        thickness=0.100,
                        is_step_down=True
                    )
                    self.zones.append(z)
                    zone_idx += 1
        except Exception as e:
            print(f"Warnung: Zonen-Import fehlgeschlagen: {e}")

    def _load_paste_gerber(self, path: str):
        """Lade Aperturen aus Paste-Gerberdatei."""
        g = GerberFile.open(path)
        bbox = g.bounding_box(unit=MM)
        self.bounds = (bbox[0][0], bbox[0][1], bbox[1][0], bbox[1][1])

        self.apertures.clear()
        self.spatial_index = index.Index()

        item_id = 0
        for obj in g.objects:
            prims = list(obj.to_primitives(unit=MM))
            for prim in prims:
                pname = type(prim).__name__
                points = []
                if pname == 'Circle':
                    d = prim.diameter if hasattr(prim, 'diameter') else prim.r * 2
                    cx, cy = prim.x, prim.y
                    w = h = d
                    area = math.pi * (d / 2) ** 2
                    perim = math.pi * d
                    shape_type = 'circle'
                    bbox = (cx - d/2, cy - d/2, cx + d/2, cy + d/2)
                elif pname == 'Rectangle':
                    w, h = prim.w, prim.h
                    cx, cy = prim.x, prim.y
                    area = w * h
                    perim = 2 * (w + h)
                    shape_type = 'rectangle'
                    bbox = (cx - w/2, cy - h/2, cx + w/2, cy + h/2)
                    points = [
                        (cx - w/2, cy - h/2),
                        (cx + w/2, cy - h/2),
                        (cx + w/2, cy + h/2),
                        (cx - w/2, cy + h/2)
                    ]
                elif pname == 'ArcPoly':
                    pts = prim.outline
                    n = len(pts)
                    if n < 3:
                        continue
                    min_x = min(p[0] for p in pts)
                    max_x = max(p[0] for p in pts)
                    min_y = min(p[1] for p in pts)
                    max_y = max(p[1] for p in pts)
                    w, h = max_x - min_x, max_y - min_y
                    cx, cy = (min_x + max_x) / 2, (min_y + max_y) / 2
                    area = 0.5 * abs(sum(pts[i][0]*pts[(i+1)%n][1] - pts[(i+1)%n][0]*pts[i][1] for i in range(n)))
                    perim = sum(math.hypot(pts[(i+1)%n][0]-pts[i][0], pts[(i+1)%n][1]-pts[i][1]) for i in range(n))
                    shape_type = 'polygon'
                    bbox = (min_x, min_y, max_x, max_y)
                    points = [(round(p[0], 4), round(p[1], 4)) for p in pts]
                else:
                    continue

                if area <= 1e-5:
                    continue

                pkg_grp = f"{round(w, 2):.2f}x{round(h, 2):.2f}"
                item = ApertureItem(
                    id=item_id,
                    cx=cx,
                    cy=cy,
                    bbox=bbox,
                    shape_type=shape_type,
                    width=w,
                    height=h,
                    area=area,
                    perimeter=perim,
                    points=points,
                    original_obj=obj,
                    package_group=pkg_grp
                )
                self.apertures.append(item)
                self.spatial_index.insert(item_id, bbox)
                item_id += 1

    def _load_pads_gerber(self, path: str):
        """Lade Kupfer-Pads für Überlagerungsprüfung (Pad vs Paste)."""
        try:
            g = GerberFile.open(path)
            for o in g.objects:
                bbox = o.bounding_box(unit=MM)
                self.pads.append({
                    "x0": bbox[0][0],
                    "y0": bbox[0][1],
                    "x1": bbox[1][0],
                    "y1": bbox[1][1],
                    "w": bbox[1][0] - bbox[0][0],
                    "h": bbox[1][1] - bbox[0][1]
                })
        except Exception as e:
            print(f"Pads konnten nicht geladen werden: {e}")

    def _load_profile_gerber(self, path: str):
        """Lade Platinen- und Nutzenkontur."""
        try:
            g = GerberFile.open(path)
            for o in g.objects:
                if hasattr(o, 'x1'):
                    self.profile_paths.append([(round(o.x1, 2), round(o.y1, 2)), (round(o.x2, 2), round(o.y2, 2))])
        except Exception as e:
            print(f"Profil konnte nicht geladen werden: {e}")

    def _load_fiducials_gerber(self, path: str):
        """Lade Passermarken."""
        try:
            g = GerberFile.open(path)
            for o in g.objects:
                bbox = o.bounding_box(unit=MM)
                cx = (bbox[0][0] + bbox[1][0]) / 2
                cy = (bbox[0][1] + bbox[1][1]) / 2
                r = (bbox[1][0] - bbox[0][0]) / 2
                self.fiducials.append((cx, cy, r))
        except Exception as e:
            print(f"Fiducials konnten nicht geladen werden: {e}")

    def get_effective_thickness(self, cx: float, cy: float) -> Tuple[float, Optional[str]]:
        """Gibt die effektive Schablonendicke an einer Position (mm) und Zone zurück."""
        for z in self.zones:
            if z.contains_point(cx, cy):
                return z.thickness, z.id
        return self.base_thickness, None

    def calculate_aperture_ipc(self, item: ApertureItem) -> Dict[str, Any]:
        """Berechnet IPC-7525 Kennzahlen für eine Apertur unter ihrer effektiven Dicke."""
        T, zone_id = self.get_effective_thickness(item.cx, item.cy)

        if item.custom_slices and len(item.custom_slices) > 0:
            seg_w = item.custom_slices[0]["w"]
            seg_h = item.custom_slices[0]["h"]
            seg_area = seg_w * seg_h
            seg_perim = 2 * (seg_w + seg_h)
            ar = seg_area / (seg_perim * T) if seg_perim > 0 else 0
            aspect = min(seg_w, seg_h) / T
        elif item.shape_type == 'circle':
            ar = item.width / (4 * T)
            aspect = item.width / T
        else:
            ar = item.area / (item.perimeter * T) if item.perimeter > 0 else 0
            aspect = min(item.width, item.height) / T

        if ar >= self.ipc_min_ar and aspect >= self.ipc_min_aspect:
            status = "OK"
        elif ar >= (self.ipc_min_ar * 0.90) and aspect >= (self.ipc_min_aspect * 0.90):
            status = "WARN"
        else:
            status = "CRIT"

        clearance_warn = False
        if zone_id is not None:
            zone = next((z for z in self.zones if z.id == zone_id), None)
            if zone:
                dx = min(abs(item.cx - zone.x0), abs(item.cx - zone.x1))
                dy = min(abs(item.cy - zone.y0), abs(item.cy - zone.y1))
                if self.squeegee_dir == "Y":
                    if dy < self.squeegee_clearance or dx < self.transverse_clearance:
                        clearance_warn = True
                else:
                    if dx < self.squeegee_clearance or dy < self.transverse_clearance:
                        clearance_warn = True

        return {
            "effective_thickness": T,
            "zone_id": zone_id,
            "area_ratio": round(ar, 3),
            "aspect_ratio": round(aspect, 2),
            "status": status,
            "clearance_warn": clearance_warn
        }

    def get_panel_statistics(self) -> Dict[str, Any]:
        """Liefert vollständige Statistiken für das Dashboard."""
        stats = {
            "total_apertures": len(self.apertures),
            "ok_count": 0,
            "warn_count": 0,
            "crit_count": 0,
            "min_ar": 99.0,
            "base_thickness": self.base_thickness,
            "zones_count": len(self.zones),
            "zones": [z.to_dict() for z in self.zones],
            "ipc_mode": self.ipc_mode,
            "ipc_min_ar": self.ipc_min_ar,
            "ipc_min_aspect": self.ipc_min_aspect,
            "squeegee_dir": self.squeegee_dir,
            "bounds": [round(b, 2) for b in self.bounds]
        }

        package_groups = Counter()
        crit_groups = Counter()

        for item in self.apertures:
            ipc = self.calculate_aperture_ipc(item)
            package_groups[item.package_group] += 1
            if ipc["status"] == "OK":
                stats["ok_count"] += 1
            elif ipc["status"] == "WARN":
                stats["warn_count"] += 1
            else:
                stats["crit_count"] += 1
                crit_groups[item.package_group] += 1

            if ipc["area_ratio"] < stats["min_ar"]:
                stats["min_ar"] = ipc["area_ratio"]

        if stats["min_ar"] > 90:
            stats["min_ar"] = 0.0
        else:
            stats["min_ar"] = round(stats["min_ar"], 3)

        stats["top_critical_packages"] = [
            {"group": grp, "crit_count": count, "total": package_groups[grp]}
            for grp, count in crit_groups.most_common(5)
        ]

        return stats

    # -------------------------------------------------------------
    # Slicing & Aperture Modification Tools
    # -------------------------------------------------------------
    def apply_thermal_slicing(self, 
                              target_id: int, 
                              grid_x: int = 2, 
                              grid_y: int = 2, 
                              web_width: float = 0.25, 
                              edge_reduction: float = 0.05,
                              apply_to_all_identical: bool = False) -> List[int]:
        if target_id < 0 or target_id >= len(self.apertures):
            return []

        target = self.apertures[target_id]
        targets = [target]
        if apply_to_all_identical:
            targets = [a for a in self.apertures if a.package_group == target.package_group]

        modified_ids = []
        for ap in targets:
            total_w = ap.width - 2 * edge_reduction
            total_h = ap.height - 2 * edge_reduction
            if total_w <= 0 or total_h <= 0:
                continue

            sub_w = (total_w - (grid_x - 1) * web_width) / grid_x
            sub_h = (total_h - (grid_y - 1) * web_width) / grid_y
            if sub_w <= 0 or sub_h <= 0:
                continue

            slices = []
            start_x = ap.cx - (total_w / 2) + (sub_w / 2)
            start_y = ap.cy - (total_h / 2) + (sub_h / 2)

            for ix in range(grid_x):
                for iy in range(grid_y):
                    cx = start_x + ix * (sub_w + web_width)
                    cy = start_y + iy * (sub_h + web_width)
                    slices.append({
                        "cx": round(cx, 4),
                        "cy": round(cy, 4),
                        "w": round(sub_w, 4),
                        "h": round(sub_h, 4)
                    })

            ap.custom_slices = slices
            modified_ids.append(ap.id)

        return modified_ids

    def apply_reduction(self, 
                        target_id: int, 
                        percent: float = 10.0, 
                        apply_to_all_identical: bool = False) -> List[int]:
        if target_id < 0 or target_id >= len(self.apertures):
            return []

        target = self.apertures[target_id]
        targets = [target]
        if apply_to_all_identical:
            targets = [a for a in self.apertures if a.package_group == target.package_group]

        scale = math.sqrt(max(0.1, 1.0 - (percent / 100.0)))
        modified_ids = []
        for ap in targets:
            new_w = ap.width * scale
            new_h = ap.height * scale
            ap.custom_slices = [{
                "cx": ap.cx,
                "cy": ap.cy,
                "w": round(new_w, 4),
                "h": round(new_h, 4)
            }]
            modified_ids.append(ap.id)

        return modified_ids

    def reset_aperture_modification(self, target_id: int, apply_to_all_identical: bool = False):
        if target_id < 0 or target_id >= len(self.apertures):
            return
        target = self.apertures[target_id]
        targets = [target]
        if apply_to_all_identical:
            targets = [a for a in self.apertures if a.package_group == target.package_group]
        for ap in targets:
            ap.custom_slices = None

    def apply_elongated_narrowing(self, 
                                  target_id: int, 
                                  width_reduction_pct: float = 20.0, 
                                  length_usage_pct: float = 95.0, 
                                  corner_radius: float = 0.08, 
                                  apply_to_all_identical: bool = False) -> List[int]:
        if target_id < 0 or target_id >= len(self.apertures):
            return []

        target = self.apertures[target_id]
        targets = [target]
        if apply_to_all_identical:
            targets = [a for a in self.apertures if a.package_group == target.package_group]

        modified_ids = []
        for ap in targets:
            w = ap.width
            h = ap.height

            if w <= h:
                new_w = w * (1.0 - (width_reduction_pct / 100.0))
                new_h = h * (length_usage_pct / 100.0)
            else:
                new_w = w * (length_usage_pct / 100.0)
                new_h = h * (1.0 - (width_reduction_pct / 100.0))

            r = min(corner_radius, new_w / 3.0, new_h / 3.0)
            cx, cy = ap.cx, ap.cy

            pts = [
                (cx - new_w/2 + r, cy + new_h/2),
                (cx + new_w/2 - r, cy + new_h/2),
                (cx + new_w/2, cy + new_h/2 - r),
                (cx + new_w/2, cy - new_h/2 + r),
                (cx + new_w/2 - r, cy - new_h/2),
                (cx - new_w/2 + r, cy - new_h/2),
                (cx - new_w/2, cy - new_h/2 + r),
                (cx - new_w/2, cy + new_h/2 - r)
            ]

            ap.custom_slices = [{
                "cx": round(cx, 4),
                "cy": round(cy, 4),
                "w": round(new_w, 4),
                "h": round(new_h, 4),
                "radius": round(r, 4),
                "pts": [(round(px, 4), round(py, 4)) for px, py in pts],
                "type": "elongated"
            }]
            modified_ids.append(ap.id)

        return modified_ids

    def calculate_transfer_calculator(self, 
                                      width: float, 
                                      height: float, 
                                      unit: str = "mil",
                                      thicknesses: Optional[List[float]] = None) -> Dict[str, Any]:
        """
        Berechnet den Pastenauslösekalkulator (Transfer Efficiency) analog der Inhouse-Excel-Tabelle.
        """
        if thicknesses is None:
            thicknesses = [80.0, 100.0, 110.0, 120.0, 150.0]

        if unit.lower() == "mil":
            w_mil = width
            h_mil = height
            w_mm = width * 0.0254
            h_mm = height * 0.0254
        else:
            w_mm = width
            h_mm = height
            w_mil = width / 0.0254
            h_mil = height / 0.0254

        area_mil = w_mil * h_mil
        perim_mil = 2.0 * (w_mil + h_mil)
        area_mm = w_mm * h_mm
        perim_mm = 2.0 * (w_mm + h_mm)

        results = []
        baseline_vol = None

        for t_um in thicknesses:
            t_mil = t_um / 25.0
            t_mm = t_um / 1000.0

            wall_mil = perim_mil * t_mil
            wall_mm = perim_mm * t_mm

            ar = area_mil / wall_mil if wall_mil > 0 else 0.0

            eff_w = max(0.0, w_mil - (0.1 * t_mil))
            eff_l = max(0.0, h_mil - (0.1 * t_mil))
            eff_area = eff_w * eff_l
            theor_vol_mil3 = eff_area * t_mil

            if ar <= 0.7:
                eta = 2.535 * (ar ** 3.426)
            elif ar >= 1.0:
                eta = 1.0
            else:
                eta = ar

            actual_vol_mil3 = theor_vol_mil3 * eta
            actual_vol_nl = actual_vol_mil3 * 0.000016387064

            if baseline_vol is None:
                baseline_vol = actual_vol_mil3

            rel_pct = round((actual_vol_mil3 / baseline_vol) * 100.0) if baseline_vol else 100
            status = "OK" if ar >= 0.70 else ("WARN" if ar >= 0.66 else "CRIT")

            results.append({
                "thickness_um": int(t_um),
                "thickness_mil": round(t_mil, 2),
                "area_mil2": round(area_mil, 2),
                "wall_mil2": round(wall_mil, 2),
                "area_mm2": round(area_mm, 4),
                "wall_mm2": round(wall_mm, 4),
                "area_ratio": round(ar, 2),
                "volume_mil3": round(actual_vol_mil3, 2),
                "volume_nl": round(actual_vol_nl, 3),
                "transfer_pct": rel_pct,
                "status": status
            })

        return {
            "input_unit": unit,
            "width_mil": round(w_mil, 2),
            "height_mil": round(h_mil, 2),
            "width_mm": round(w_mm, 3),
            "height_mm": round(h_mm, 3),
            "results": results
        }

    # -------------------------------------------------------------
    # Stufenzonen Management
    # -------------------------------------------------------------
    def add_zone(self, x0: float, y0: float, x1: float, y1: float, thickness: float, name: str = "") -> StencilZone:
        zone_id = f"zone_{len(self.zones) + 1}"
        if not name:
            is_step_down = thickness < self.base_thickness
            type_str = "Step-Down" if is_step_down else "Step-Up"
            name = f"Zone {len(self.zones)+1} ({type_str} {int(thickness*1000)}µm)"

        z = StencilZone(
            id=zone_id,
            name=name,
            x0=min(x0, x1),
            y0=min(y0, y1),
            x1=max(x0, x1),
            y1=max(y0, y1),
            thickness=thickness,
            is_step_down=(thickness < self.base_thickness)
        )
        self.zones.append(z)
        return z

    def update_zone(self, zone_id: str, thickness: float, name: Optional[str] = None):
        for z in self.zones:
            if z.id == zone_id:
                z.thickness = thickness
                z.is_step_down = (thickness < self.base_thickness)
                if name:
                    z.name = name
                break

    def delete_zone(self, zone_id: str):
        self.zones = [z for z in self.zones if z.id != zone_id]

    # -------------------------------------------------------------
    # Gerber Export
    # -------------------------------------------------------------
    def export_modified_gerber(self, output_path: str):
        """Exportiert alle (modifizierten & originalen) Aperturen als sauberes Gerber."""
        out_g = GerberFile()

        for ap in self.apertures:
            if ap.custom_slices and len(ap.custom_slices) > 0:
                for s in ap.custom_slices:
                    if "pts" in s and s["pts"]:
                        reg = Region(outline=s["pts"], unit=MM, polarity_dark=True)
                        out_g.objects.append(reg)
                    else:
                        rect_ap = RectangleAperture(s["w"], s["h"], unit=MM)
                        out_g.objects.append(Flash(s["cx"], s["cy"], aperture=rect_ap, unit=MM))
            elif ap.shape_type == 'circle':
                circ_ap = CircleAperture(ap.width, unit=MM)
                out_g.objects.append(Flash(ap.cx, ap.cy, aperture=circ_ap, unit=MM))
            elif ap.shape_type == 'rectangle':
                rect_ap = RectangleAperture(ap.width, ap.height, unit=MM)
                out_g.objects.append(Flash(ap.cx, ap.cy, aperture=rect_ap, unit=MM))
            elif ap.shape_type == 'polygon' and ap.points:
                reg = Region(outline=ap.points, unit=MM, polarity_dark=True)
                out_g.objects.append(reg)
            else:
                if ap.original_obj:
                    out_g.objects.append(ap.original_obj)

        os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
        out_g.save(output_path)
        return output_path

    def export_zones_gerber(self, output_path: str):
        """Exportiert Stufenzonen als Fräs-/Ätz-Layer für den Schablonenhersteller."""
        out_g = GerberFile()
        for z in self.zones:
            reg = Region.from_rectangle(z.x0, z.y0, z.width, z.height, unit=MM)
            out_g.objects.append(reg)

        os.makedirs(os.path.dirname(os.path.abspath(output_path)), exist_ok=True)
        out_g.save(output_path)
        return output_path

    # -------------------------------------------------------------
    # Package Management, Detection & Stencil Generator
    # -------------------------------------------------------------
    def init_standard_packages(self):
        """Initialisiert Standard-Packages (0201 bis 2512, QFN, SOIC, BGA) mit relativen Padkoordinaten."""
        self.packages.clear()
        
        self.packages["0201"] = PackageTemplate(
            name="0201", pin_count=2, pitch=0.50, pad_width=0.30, pad_height=0.30,
            category="SMD Passives", outline={"w": 0.60, "h": 0.30},
            pads_relative=[
                {"pin": 1, "cx": -0.25, "cy": 0.0, "w": 0.30, "h": 0.30, "facing": "right"},
                {"pin": 2, "cx": 0.25, "cy": 0.0, "w": 0.30, "h": 0.30, "facing": "left"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.015, undersize_h=0.015, corner_radius=0.04)
        )

        self.packages["0402"] = PackageTemplate(
            name="0402", pin_count=2, pitch=0.90, pad_width=0.50, pad_height=0.50,
            category="SMD Passives", outline={"w": 1.00, "h": 0.50},
            pads_relative=[
                {"pin": 1, "cx": -0.45, "cy": 0.0, "w": 0.50, "h": 0.50, "facing": "right"},
                {"pin": 2, "cx": 0.45, "cy": 0.0, "w": 0.50, "h": 0.50, "facing": "left"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.02, undersize_h=0.02, corner_radius=0.05)
        )

        self.packages["0603"] = PackageTemplate(
            name="0603", pin_count=2, pitch=1.30, pad_width=0.80, pad_height=0.80,
            category="SMD Passives", outline={"w": 1.60, "h": 0.80},
            pads_relative=[
                {"pin": 1, "cx": -0.65, "cy": 0.0, "w": 0.80, "h": 0.80, "facing": "right"},
                {"pin": 2, "cx": 0.65, "cy": 0.0, "w": 0.80, "h": 0.80, "facing": "left"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.025, undersize_h=0.025, corner_radius=0.06)
        )

        self.packages["0805"] = PackageTemplate(
            name="0805", pin_count=2, pitch=1.702, pad_width=1.20, pad_height=1.30,
            category="SMD Passives", outline={"w": 2.00, "h": 1.25},
            pads_relative=[
                {"pin": 1, "cx": -0.851, "cy": 0.0, "w": 1.20, "h": 1.30, "facing": "right"},
                {"pin": 2, "cx": 0.851, "cy": 0.0, "w": 1.20, "h": 1.30, "facing": "left"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.025, undersize_h=0.025, corner_radius=0.075)
        )

        self.packages["1206"] = PackageTemplate(
            name="1206", pin_count=2, pitch=2.80, pad_width=1.60, pad_height=1.50,
            category="SMD Passives", outline={"w": 3.20, "h": 1.60},
            pads_relative=[
                {"pin": 1, "cx": -1.40, "cy": 0.0, "w": 1.60, "h": 1.50, "facing": "right"},
                {"pin": 2, "cx": 1.40, "cy": 0.0, "w": 1.60, "h": 1.50, "facing": "left"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.03, undersize_h=0.03, corner_radius=0.10)
        )

        self.packages["SOT-23"] = PackageTemplate(
            name="SOT-23", pin_count=3, pitch=0.95, pad_width=0.60, pad_height=0.80,
            category="ICs & Transistors", outline={"w": 2.90, "h": 1.30},
            pads_relative=[
                {"pin": 1, "cx": -0.95, "cy": -1.0, "w": 0.60, "h": 0.80, "facing": "up"},
                {"pin": 2, "cx": 0.95, "cy": -1.0, "w": 0.60, "h": 0.80, "facing": "up"},
                {"pin": 3, "cx": 0.0, "cy": 1.0, "w": 0.60, "h": 0.80, "facing": "down"}
            ],
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.02, undersize_h=0.02, corner_radius=0.05)
        )

        soic8_pads = []
        for i in range(4):
            x = -1.905 + i * 1.27
            soic8_pads.append({"pin": i+1, "cx": x, "cy": -2.2, "w": 0.60, "h": 1.50, "facing": "up"})
        for i in range(4):
            x = 1.905 - i * 1.27
            soic8_pads.append({"pin": i+5, "cx": x, "cy": 2.2, "w": 0.60, "h": 1.50, "facing": "down"})
        self.packages["SOIC-8"] = PackageTemplate(
            name="SOIC-8", pin_count=8, pitch=1.27, pad_width=0.60, pad_height=1.50,
            category="ICs & Transistors", outline={"w": 4.90, "h": 3.90},
            pads_relative=soic8_pads,
            rule=PackageRule(name="Länglich & Schmal (Pitch-Schutz)", rule_type="elongated_pitch", pitch_reduction_pct=20.0, length_usage_pct=95.0, corner_radius=0.08)
        )

        soic16_pads = []
        for i in range(8):
            x = -4.445 + i * 1.27
            soic16_pads.append({"pin": i+1, "cx": x, "cy": -2.2, "w": 0.60, "h": 1.50, "facing": "up"})
        for i in range(8):
            x = 4.445 - i * 1.27
            soic16_pads.append({"pin": i+9, "cx": x, "cy": 2.2, "w": 0.60, "h": 1.50, "facing": "down"})
        self.packages["SOIC-16"] = PackageTemplate(
            name="SOIC-16", pin_count=16, pitch=1.27, pad_width=0.60, pad_height=1.50,
            category="ICs & Transistors", outline={"w": 9.90, "h": 3.90},
            pads_relative=soic16_pads,
            rule=PackageRule(name="Länglich & Schmal (Pitch-Schutz)", rule_type="elongated_pitch", pitch_reduction_pct=20.0, length_usage_pct=95.0, corner_radius=0.08)
        )

        tssop16_pads = []
        for i in range(8):
            x = -2.275 + i * 0.65
            tssop16_pads.append({"pin": i+1, "cx": x, "cy": -2.7, "w": 0.35, "h": 1.20, "facing": "up"})
        for i in range(8):
            x = 2.275 - i * 0.65
            tssop16_pads.append({"pin": i+9, "cx": x, "cy": 2.7, "w": 0.35, "h": 1.20, "facing": "down"})
        self.packages["TSSOP-16"] = PackageTemplate(
            name="TSSOP-16", pin_count=16, pitch=0.65, pad_width=0.35, pad_height=1.20,
            category="ICs & Transistors", outline={"w": 5.00, "h": 4.40},
            pads_relative=tssop16_pads,
            rule=PackageRule(name="Länglich & Schmal (Pitch-Schutz)", rule_type="elongated_pitch", pitch_reduction_pct=20.0, length_usage_pct=95.0, corner_radius=0.06)
        )

        qfn48_pads = []
        pin_idx = 1
        for i in range(12):
            y = 2.75 - i * 0.5
            qfn48_pads.append({"pin": pin_idx, "cx": -3.4, "cy": y, "w": 0.80, "h": 0.28, "facing": "right"})
            pin_idx += 1
        for i in range(12):
            x = -2.75 + i * 0.5
            qfn48_pads.append({"pin": pin_idx, "cx": x, "cy": -3.4, "w": 0.28, "h": 0.80, "facing": "up"})
            pin_idx += 1
        for i in range(12):
            y = -2.75 + i * 0.5
            qfn48_pads.append({"pin": pin_idx, "cx": 3.4, "cy": y, "w": 0.80, "h": 0.28, "facing": "left"})
            pin_idx += 1
        for i in range(12):
            x = 2.75 - i * 0.5
            qfn48_pads.append({"pin": pin_idx, "cx": x, "cy": 3.4, "w": 0.28, "h": 0.80, "facing": "down"})
            pin_idx += 1
        qfn48_pads.append({"pin": 49, "cx": 0.0, "cy": 0.0, "w": 4.50, "h": 4.50, "facing": "center", "is_thermal": True})

        self.packages["QFN-48"] = PackageTemplate(
            name="QFN-48", pin_count=49, pitch=0.50, pad_width=0.28, pad_height=0.80,
            category="ICs & Transistors", outline={"w": 7.00, "h": 7.00},
            pads_relative=qfn48_pads,
            rule=PackageRule(name="HouseBuilder", rule_type="house_builder", undersize_w=0.025, undersize_h=0.025, house_cut_pct=35.0)
        )

        bga_pads = []
        bga_pin = 1
        for row in range(15):
            for col in range(15):
                if (row < 2 or row > 12) and (col < 2 or col > 12):
                    continue
                x = (col - 7) * 0.65
                y = (row - 7) * 0.65
                bga_pads.append({"pin": bga_pin, "cx": round(x, 3), "cy": round(y, 3), "w": 0.25, "h": 0.25, "facing": "center"})
                bga_pin += 1
                if bga_pin > 201:
                    break

        self.packages["BGA-201"] = PackageTemplate(
            name="BGA-201", pin_count=len(bga_pads), pitch=0.65, pad_width=0.25, pad_height=0.25,
            category="BGA & Arrays", outline={"w": 10.0, "h": 10.0},
            pads_relative=bga_pads,
            rule=PackageRule(name="Simple Rectangle Surface", undersize_w=0.015, undersize_h=0.015, corner_radius=0.05)
        )

        self.package_aliases = {
            "R0201": "0201", "C0201": "0201",
            "R0402": "0402", "C0402": "0402", "C0402X": "0402",
            "R0603": "0603", "C0603": "0603", "BLM18A": "0603", "BLM18P": "0603", "BLM18PG": "0603",
            "R0805": "0805", "C0805": "0805", "DLW210-0805": "0805",
            "R1206": "1206", "C1206": "1206",
            "BGA201": "BGA-201", "BGA201C65P15X15_1000X1000X60": "BGA-201"
        }

    def detect_packages(self):
        """Scannt Aperturen und ordnet sie präzise Footprints zu."""
        if not self.apertures:
            return

        for pkg in self.packages.values():
            pkg.component_count = 0

        used_ids = set()
        comp_id_counter = 1

        small_pads = [ap for ap in self.apertures if max(ap.width, ap.height) <= 0.35 and ap.id not in used_ids]
        if len(small_pads) >= 50:
            bga_group = small_pads
            if len(bga_group) == 201 or len(bga_group) >= 150:
                for ap in bga_group:
                    ap.package_name = "BGA-201"
                    ap.component_id = comp_id_counter
                    used_ids.add(ap.id)
                comp_id_counter += 1
                if "BGA-201" in self.packages:
                    self.packages["BGA-201"].component_count += 1

        for ap in self.apertures:
            if ap.id in used_ids:
                continue

            minx, miny, maxx, maxy = ap.cx - 3.2, ap.cy - 3.2, ap.cx + 3.2, ap.cy + 3.2
            best_dist = 999.0
            best_id = None
            for nid in self.spatial_index.intersection((minx, miny, maxx, maxy)):
                if nid == ap.id or nid in used_ids:
                    continue
                nap = self.apertures[nid]
                if abs(nap.width - ap.width) < 0.08 and abs(nap.height - ap.height) < 0.08:
                    d = math.hypot(nap.cx - ap.cx, nap.cy - ap.cy)
                    if 0.35 <= d < best_dist:
                        best_dist = d
                        best_id = nid

            if best_id is not None and best_dist <= 3.2:
                nap = self.apertures[best_id]
                n_best_dist = 999.0
                n_best_id = None
                for nid in self.spatial_index.intersection((nap.cx - 3.2, nap.cy - 3.2, nap.cx + 3.2, nap.cy + 3.2)):
                    if nid == best_id or nid in used_ids:
                        continue
                    oap = self.apertures[nid]
                    if abs(oap.width - nap.width) < 0.08 and abs(oap.height - nap.height) < 0.08:
                        d = math.hypot(oap.cx - nap.cx, oap.cy - nap.cy)
                        if 0.35 <= d < n_best_dist:
                            n_best_dist = d
                            n_best_id = nid

                if n_best_id == ap.id:
                    used_ids.add(ap.id)
                    used_ids.add(best_id)

                    pitch = best_dist
                    half_p = round(pitch / 2.0, 3)
                    pw = round(ap.width, 3)
                    ph = round(ap.height, 3)
                    if pitch < 0.70:
                        pkg_name = "0201"
                    elif pitch < 1.05:
                        pkg_name = "0402"
                    elif pitch < 1.50:
                        pkg_name = "0603"
                    elif pitch < 2.20:
                        pkg_name = "0805"
                    else:
                        pkg_name = "1206"

                    dx = nap.cx - ap.cx
                    dy = nap.cy - ap.cy
                    if abs(dx) >= abs(dy):
                        if dx > 0:
                            ap.facing_dir = "right"
                            nap.facing_dir = "left"
                        else:
                            ap.facing_dir = "left"
                            nap.facing_dir = "right"
                    else:
                        if dy > 0:
                            ap.facing_dir = "up"
                            nap.facing_dir = "down"
                        else:
                            ap.facing_dir = "down"
                            nap.facing_dir = "up"

                    # Setze tatsächliche relative Geometrie für dieses 2-Pin Package
                    if pkg_name in self.packages:
                        self.packages[pkg_name].pad_width = pw
                        self.packages[pkg_name].pad_height = ph
                        self.packages[pkg_name].pitch = round(pitch, 3)
                        self.packages[pkg_name].pads_relative = [
                            {"pin": 1, "cx": -half_p, "cy": 0.0, "w": pw, "h": ph, "facing": "right"},
                            {"pin": 2, "cx": half_p, "cy": 0.0, "w": pw, "h": ph, "facing": "left"}
                        ]
                        self.packages[pkg_name].outline = {"w": round(pitch + pw + 0.4, 2), "h": round(ph * 1.3, 2)}

                    ap.package_name = pkg_name
                    ap.component_id = comp_id_counter
                    ap.pin_number = 1

                    nap.package_name = pkg_name
                    nap.component_id = comp_id_counter
                    nap.pin_number = 2

                    comp_id_counter += 1
                    if pkg_name in self.packages:
                        self.packages[pkg_name].component_count += 1

        remaining = [ap for ap in self.apertures if ap.id not in used_ids]
        clusters = []
        cluster_visited = set()
        for ap in remaining:
            if ap.id in cluster_visited:
                continue
            comp = []
            queue = [ap.id]
            cluster_visited.add(ap.id)
            while queue:
                cid = queue.pop(0)
                cap = self.apertures[cid]
                comp.append(cap)
                minx, miny, maxx, maxy = cap.cx - 2.5, cap.cy - 2.5, cap.cx + 2.5, cap.cy + 2.5
                for nid in self.spatial_index.intersection((minx, miny, maxx, maxy)):
                    if nid not in cluster_visited and nid not in used_ids:
                        nap = self.apertures[nid]
                        if math.hypot(nap.cx - cap.cx, nap.cy - cap.cy) <= 2.5:
                            cluster_visited.add(nid)
                            queue.append(nid)
            clusters.append(comp)

        for comp in clusters:
            pins = len(comp)
            if pins == 8:
                pkg_name = "SOIC-8"
                category = "ICs & Transistors"
            elif pins == 16:
                pkg_name = "SOIC-16"
                category = "ICs & Transistors"
            elif pins in (24, 25):
                pkg_name = "QFN-24"
                category = "ICs & Transistors"
            elif pins in (32, 33):
                pkg_name = "QFN-32"
                category = "ICs & Transistors"
            elif pins in (48, 49):
                pkg_name = "QFN-48"
                category = "ICs & Transistors"
            elif pins >= 3:
                pkg_name = f"Generic_{pins}Pin"
                category = "Connectors" if pins in (10, 12, 14, 20) else "ICs & Transistors"
            else:
                continue

            # Exakte Bounding Box & Zentrum der Bauteil-Aperturen berechnen
            c_minx = min(a.cx - a.width/2 for a in comp)
            c_maxx = max(a.cx + a.width/2 for a in comp)
            c_miny = min(a.cy - a.height/2 for a in comp)
            c_maxy = max(a.cy + a.height/2 for a in comp)

            mcx = (c_minx + c_maxx) / 2.0
            mcy = (c_miny + c_maxy) / 2.0
            comp_w = max(0.5, c_maxx - c_minx)
            comp_h = max(0.5, c_maxy - c_miny)

            # Sortiere Pins logisch: von oben nach unten, dann links nach rechts
            comp_sorted = sorted(comp, key=lambda a: (-round(a.cy, 2), round(a.cx, 2)))

            pads_rel = []
            for i, ap in enumerate(comp_sorted):
                adx = mcx - ap.cx
                ady = mcy - ap.cy
                if abs(adx) >= abs(ady):
                    ap.facing_dir = "right" if adx > 0 else "left"
                else:
                    ap.facing_dir = "up" if ady > 0 else "down"

                pads_rel.append({
                    "pin": i + 1,
                    "cx": round(ap.cx - mcx, 3),
                    "cy": round(ap.cy - mcy, 3),
                    "w": round(ap.width, 3),
                    "h": round(ap.height, 3),
                    "facing": ap.facing_dir
                })

            if pkg_name not in self.packages:
                avg_w = sum(a.width for a in comp) / pins
                avg_h = sum(a.height for a in comp) / pins
                self.packages[pkg_name] = PackageTemplate(
                    name=pkg_name,
                    pin_count=pins,
                    pitch=round(max(abs(pads_rel[0]["cx"] - pads_rel[1]["cx"]) if len(pads_rel) > 1 else 0.8, 0.4), 3),
                    pad_width=round(avg_w, 2),
                    pad_height=round(avg_h, 2),
                    category=category,
                    outline={"w": round(comp_w + 0.6, 2), "h": round(comp_h + 0.6, 2)},
                    pads_relative=pads_rel
                )
            else:
                self.packages[pkg_name].pads_relative = pads_rel
                self.packages[pkg_name].outline = {"w": round(comp_w + 0.6, 2), "h": round(comp_h + 0.6, 2)}

            for i, ap in enumerate(comp_sorted):
                ap.package_name = pkg_name
                ap.component_id = comp_id_counter
                ap.pin_number = i + 1
                used_ids.add(ap.id)

            comp_id_counter += 1
            if pkg_name in self.packages:
                self.packages[pkg_name].component_count += 1

        for ap in self.apertures:
            if ap.id not in used_ids:
                grp_name = f"Generic_{round(ap.width, 2):.2f}x{round(ap.height, 2):.2f}"
                ap.package_name = grp_name
                ap.component_id = comp_id_counter
                comp_id_counter += 1
                if grp_name not in self.packages:
                    self.packages[grp_name] = PackageTemplate(
                        name=grp_name, pin_count=1, pitch=0.0,
                        pad_width=ap.width, pad_height=ap.height,
                        category="Other",
                        pads_relative=[{"pin": 1, "cx": 0.0, "cy": 0.0, "w": ap.width, "h": ap.height, "facing": "center"}]
                    )
                self.packages[grp_name].component_count += 1

    def generate_rule_aperture_geometry(self, pad_w: float, pad_h: float, pad_cx: float, pad_cy: float, rule: PackageRule, facing_dir: str = 'right') -> Optional[List[Dict[str, Any]]]:
        """Erzeugt Apertur-Geometrie für Simple Rectangle, HouseBuilder, InvertedHouseBuilder, etc."""
        rtype = rule.rule_type

        if rtype == "take_existing":
            return None

        if rtype == "ignore":
            return []

        w_eff = max(0.04, pad_w - 2.0 * rule.undersize_w)
        h_eff = max(0.04, pad_h - 2.0 * rule.undersize_h)

        cx = pad_cx + rule.offset_x
        cy = pad_cy + rule.offset_y

        if rule.offset_move_away != 0.0:
            if facing_dir == "right": cx -= rule.offset_move_away
            elif facing_dir == "left": cx += rule.offset_move_away
            elif facing_dir == "up": cy -= rule.offset_move_away
            elif facing_dir == "down": cy += rule.offset_move_away

        if rtype == "simple_rect":
            r = min(rule.corner_radius, w_eff / 2.2, h_eff / 2.2)
            if r > 0.01:
                pts = [
                    (cx - w_eff/2 + r, cy + h_eff/2),
                    (cx + w_eff/2 - r, cy + h_eff/2),
                    (cx + w_eff/2, cy + h_eff/2 - r),
                    (cx + w_eff/2, cy - h_eff/2 + r),
                    (cx + w_eff/2 - r, cy - h_eff/2),
                    (cx - w_eff/2 + r, cy - h_eff/2),
                    (cx - w_eff/2, cy - h_eff/2 + r),
                    (cx - w_eff/2, cy + h_eff/2 - r)
                ]
                return [{
                    "cx": round(cx, 4), "cy": round(cy, 4),
                    "w": round(w_eff, 4), "h": round(h_eff, 4),
                    "radius": round(r, 4),
                    "pts": [(round(px, 4), round(py, 4)) for px, py in pts],
                    "shape": "polygon", "type": "simple_rect"
                }]
            else:
                return [{
                    "cx": round(cx, 4), "cy": round(cy, 4),
                    "w": round(w_eff, 4), "h": round(h_eff, 4),
                    "shape": "rectangle", "type": "simple_rect"
                }]

        elif rtype == "house_builder":
            cut_w = w_eff * (rule.house_cut_pct / 100.0)
            cut_h = h_eff * (rule.house_cut_pct / 100.0)
            half_w, half_h = w_eff / 2.0, h_eff / 2.0

            if facing_dir == "right":
                pts = [
                    (cx - half_w, cy - half_h),
                    (cx + half_w - cut_w, cy - half_h),
                    (cx + half_w, cy),
                    (cx + half_w - cut_w, cy + half_h),
                    (cx - half_w, cy + half_h)
                ]
            elif facing_dir == "left":
                pts = [
                    (cx - half_w + cut_w, cy - half_h),
                    (cx + half_w, cy - half_h),
                    (cx + half_w, cy + half_h),
                    (cx - half_w + cut_w, cy + half_h),
                    (cx - half_w, cy)
                ]
            elif facing_dir == "down":
                pts = [
                    (cx - half_w, cy + half_h),
                    (cx + half_w, cy + half_h),
                    (cx + half_w, cy - half_h + cut_h),
                    (cx, cy - half_h),
                    (cx - half_w, cy - half_h + cut_h)
                ]
            else:
                pts = [
                    (cx - half_w, cy - half_h),
                    (cx - half_w, cy + half_h - cut_h),
                    (cx, cy + half_h),
                    (cx + half_w, cy + half_h - cut_h),
                    (cx + half_w, cy - half_h)
                ]
            return [{
                "cx": round(cx, 4), "cy": round(cy, 4),
                "w": round(w_eff, 4), "h": round(h_eff, 4),
                "pts": [(round(px, 4), round(py, 4)) for px, py in pts],
                "shape": "polygon", "type": "house_builder"
            }]

        elif rtype == "inverted_house_builder":
            cut_w = w_eff * (rule.house_cut_pct / 100.0)
            cut_h = h_eff * (rule.house_cut_pct / 100.0)
            half_w, half_h = w_eff / 2.0, h_eff / 2.0

            if facing_dir == "right":
                pts = [
                    (cx + half_w, cy - half_h),
                    (cx - half_w + cut_w, cy - half_h),
                    (cx - half_w, cy),
                    (cx - half_w + cut_w, cy + half_h),
                    (cx + half_w, cy + half_h)
                ]
            elif facing_dir == "left":
                pts = [
                    (cx - half_w, cy - half_h),
                    (cx + half_w - cut_w, cy - half_h),
                    (cx + half_w, cy),
                    (cx + half_w - cut_w, cy + half_h),
                    (cx - half_w, cy + half_h)
                ]
            elif facing_dir == "up":
                pts = [
                    (cx - half_w, cy + half_h),
                    (cx + half_w, cy + half_h),
                    (cx + half_w, cy - half_h + cut_h),
                    (cx, cy - half_h),
                    (cx - half_w, cy - half_h + cut_h)
                ]
            else:  # down
                pts = [
                    (cx - half_w, cy - half_h),
                    (cx - half_w, cy + half_h - cut_h),
                    (cx, cy + half_h),
                    (cx + half_w, cy + half_h - cut_h),
                    (cx + half_w, cy - half_h)
                ]
            return [{
                "cx": round(cx, 4), "cy": round(cy, 4),
                "w": round(w_eff, 4), "h": round(h_eff, 4),
                "pts": [(round(px, 4), round(py, 4)) for px, py in pts],
                "shape": "polygon", "type": "inverted_house_builder"
            }]

        elif rtype == "elongated_pitch":
            if pad_w <= pad_h:
                new_w = pad_w * (1.0 - (rule.pitch_reduction_pct / 100.0))
                new_h = pad_h * (rule.length_usage_pct / 100.0)
            else:
                new_w = pad_w * (rule.length_usage_pct / 100.0)
                new_h = pad_h * (1.0 - (rule.pitch_reduction_pct / 100.0))

            r = min(rule.corner_radius, new_w / 2.5, new_h / 2.5)
            pts = [
                (cx - new_w/2 + r, cy + new_h/2),
                (cx + new_w/2 - r, cy + new_h/2),
                (cx + new_w/2, cy + new_h/2 - r),
                (cx + new_w/2, cy - new_h/2 + r),
                (cx + new_w/2 - r, cy - new_h/2),
                (cx - new_w/2 + r, cy - new_h/2),
                (cx - new_w/2, cy - new_h/2 + r),
                (cx - new_w/2, cy + new_h/2 - r)
            ]
            return [{
                "cx": round(cx, 4), "cy": round(cy, 4),
                "w": round(new_w, 4), "h": round(new_h, 4),
                "radius": round(r, 4),
                "pts": [(round(px, 4), round(py, 4)) for px, py in pts],
                "shape": "polygon", "type": "elongated_pitch"
            }]

        elif rtype in ("array_slicing", "array_pads"):
            gx = max(1, rule.bridge_count_x + 1)
            gy = max(1, rule.bridge_count_y + 1)
            web = max(0.1, rule.bridge_width)
            sub_w = (w_eff - (gx - 1) * web) / gx
            sub_h = (h_eff - (gy - 1) * web) / gy
            if sub_w <= 0 or sub_h <= 0:
                return [{"cx": round(cx, 4), "cy": round(cy, 4), "w": round(w_eff, 4), "h": round(h_eff, 4), "shape": "rectangle"}]

            slices = []
            start_x = cx - (w_eff / 2) + (sub_w / 2)
            start_y = cy - (h_eff / 2) + (sub_h / 2)
            for ix in range(gx):
                for iy in range(gy):
                    scx = start_x + ix * (sub_w + web)
                    scy = start_y + iy * (sub_h + web)
                    slices.append({
                        "cx": round(scx, 4), "cy": round(scy, 4),
                        "w": round(sub_w, 4), "h": round(sub_h, 4),
                        "shape": "rectangle", "type": "array_slicing"
                    })
            return slices

        return [{
            "cx": round(cx, 4), "cy": round(cy, 4),
            "w": round(w_eff, 4), "h": round(h_eff, 4),
            "shape": "rectangle"
        }]

    def _resolve_pin_rule(self, base_rule: PackageRule, pin_data: Dict[str, Any]) -> PackageRule:
        rtype = pin_data.get("rule_type", "inherit")
        if not rtype or rtype == "inherit":
            rtype = base_rule.rule_type

        return PackageRule(
            name=rtype,
            rule_type=rtype,
            undersize_w=float(pin_data.get("undersize_w", base_rule.undersize_w)),
            undersize_h=float(pin_data.get("undersize_h", base_rule.undersize_h)),
            corner_radius=float(pin_data.get("corner_radius", base_rule.corner_radius)),
            offset_x=float(pin_data.get("offset_x", base_rule.offset_x)),
            offset_y=float(pin_data.get("offset_y", base_rule.offset_y)),
            house_cut_pct=float(pin_data.get("house_cut_pct", base_rule.house_cut_pct)),
            pitch_reduction_pct=float(pin_data.get("pitch_reduction_pct", base_rule.pitch_reduction_pct)),
            length_usage_pct=float(pin_data.get("length_usage_pct", base_rule.length_usage_pct)),
            bridge_count_x=int(pin_data.get("bridge_count_x", base_rule.bridge_count_x)),
            bridge_count_y=int(pin_data.get("bridge_count_y", base_rule.bridge_count_y)),
            bridge_width=float(pin_data.get("bridge_width", base_rule.bridge_width)),
            donut_inner_dia=float(pin_data.get("donut_inner_dia", base_rule.donut_inner_dia)),
            rotate_bridges_45=bool(pin_data.get("rotate_bridges_45", base_rule.rotate_bridges_45)),
            try_find_direction=bool(pin_data.get("try_find_direction", base_rule.try_find_direction))
        )

    def apply_package_rule(self, pkg_name: str, rule_dict: Dict[str, Any]) -> int:
        target_name = self.package_aliases.get(pkg_name, pkg_name)
        if target_name not in self.packages:
            self.packages[target_name] = PackageTemplate(
                name=target_name, pin_count=2, pitch=1.0, pad_width=1.0, pad_height=1.0
            )

        pkg = self.packages[target_name]
        for k, v in rule_dict.items():
            if hasattr(pkg.rule, k):
                setattr(pkg.rule, k, v)

        modified_count = 0
        for ap in self.apertures:
            cur_ap_pkg = self.package_aliases.get(ap.package_name, ap.package_name)
            if cur_ap_pkg == target_name:
                if ap.is_manually_overridden:
                    continue
                pin = ap.pin_number
                if pin in pkg.pad_overrides:
                    pin_rule = self._resolve_pin_rule(pkg.rule, pkg.pad_overrides[pin])
                    pw = float(pkg.pad_overrides[pin].get("w", ap.width))
                    ph = float(pkg.pad_overrides[pin].get("h", ap.height))
                    ap.custom_slices = self.generate_rule_aperture_geometry(
                        pad_w=pw,
                        pad_h=ph,
                        pad_cx=ap.cx,
                        pad_cy=ap.cy,
                        rule=pin_rule,
                        facing_dir=ap.facing_dir
                    )
                else:
                    ap.custom_slices = self.generate_rule_aperture_geometry(
                        pad_w=ap.width,
                        pad_h=ap.height,
                        pad_cx=ap.cx,
                        pad_cy=ap.cy,
                        rule=pkg.rule,
                        facing_dir=ap.facing_dir
                    )
                modified_count += 1

        return modified_count

    def override_package_pin(self, pkg_name: str, pin_num: int, override_dict: Dict[str, Any]) -> bool:
        target_name = self.package_aliases.get(pkg_name, pkg_name)
        if target_name not in self.packages:
            return False

        pkg = self.packages[target_name]
        pin_int = int(pin_num)
        pkg.pad_overrides[pin_int] = override_dict

        pin_rule = self._resolve_pin_rule(pkg.rule, override_dict)
        for ap in self.apertures:
            cur_ap_pkg = self.package_aliases.get(ap.package_name, ap.package_name)
            if cur_ap_pkg == target_name and ap.pin_number == pin_int:
                pw = float(override_dict.get("w", ap.width))
                ph = float(override_dict.get("h", ap.height))
                ap.custom_slices = self.generate_rule_aperture_geometry(
                    pad_w=pw,
                    pad_h=ph,
                    pad_cx=ap.cx,
                    pad_cy=ap.cy,
                    rule=pin_rule,
                    facing_dir=ap.facing_dir
                )
        return True

    def reset_package_pin(self, pkg_name: str, pin_num: int) -> bool:
        target_name = self.package_aliases.get(pkg_name, pkg_name)
        if target_name not in self.packages:
            return False

        pkg = self.packages[target_name]
        pin_int = int(pin_num)
        if pin_int in pkg.pad_overrides:
            del pkg.pad_overrides[pin_int]

        for ap in self.apertures:
            cur_ap_pkg = self.package_aliases.get(ap.package_name, ap.package_name)
            if cur_ap_pkg == target_name and ap.pin_number == pin_int:
                ap.custom_slices = self.generate_rule_aperture_geometry(
                    pad_w=ap.width,
                    pad_h=ap.height,
                    pad_cx=ap.cx,
                    pad_cy=ap.cy,
                    rule=pkg.rule,
                    facing_dir=ap.facing_dir
                )
        return True

    def rename_package(self, old_name: str, new_name: str) -> bool:
        new_name = new_name.strip()
        if not new_name or old_name == new_name:
            return False

        real_old = self.package_aliases.get(old_name, old_name)
        if real_old in self.packages:
            pkg = self.packages.pop(real_old)
            pkg.name = new_name
            self.packages[new_name] = pkg

        self.package_aliases[old_name] = new_name
        self.package_aliases[real_old] = new_name

        for ap in self.apertures:
            if ap.package_name in (old_name, real_old):
                ap.package_name = new_name

        return True

    def close_dataset(self) -> bool:
        self.apertures.clear()
        self.zones.clear()
        self.pads.clear()
        self.components.clear()
        self.profile_paths.clear()
        self.fiducials.clear()
        self.layers.clear()
        self.packages.clear()
        self.package_aliases.clear()
        self.spatial_index = index.Index()
        self.bounds = (0.0, 0.0, 0.0, 0.0)
        self.init_standard_packages()
        return True

    def override_single_pad(self, ap_id: int, custom_data: Dict[str, Any]) -> bool:
        if ap_id < 0 or ap_id >= len(self.apertures):
            return False

        ap = self.apertures[ap_id]
        ap.is_manually_overridden = True

        if "slices" in custom_data:
            ap.custom_slices = custom_data["slices"]
        elif "w" in custom_data and "h" in custom_data:
            ap.custom_slices = [{
                "cx": ap.cx,
                "cy": ap.cy,
                "w": round(custom_data["w"], 4),
                "h": round(custom_data["h"], 4),
                "shape": "rectangle",
                "manual": True
            }]
        return True

    def reset_single_pad(self, ap_id: int) -> bool:
        if ap_id < 0 or ap_id >= len(self.apertures):
            return False

        ap = self.apertures[ap_id]
        ap.is_manually_overridden = False

        target_pkg = self.package_aliases.get(ap.package_name, ap.package_name)
        if target_pkg in self.packages:
            pkg = self.packages[target_pkg]
            pin = ap.pin_number
            if pin in pkg.pad_overrides:
                pin_rule = self._resolve_pin_rule(pkg.rule, pkg.pad_overrides[pin])
                pw = float(pkg.pad_overrides[pin].get("w", ap.width))
                ph = float(pkg.pad_overrides[pin].get("h", ap.height))
                ap.custom_slices = self.generate_rule_aperture_geometry(
                    pad_w=pw,
                    pad_h=ph,
                    pad_cx=ap.cx,
                    pad_cy=ap.cy,
                    rule=pin_rule,
                    facing_dir=ap.facing_dir
                )
            else:
                ap.custom_slices = self.generate_rule_aperture_geometry(
                    pad_w=ap.width,
                    pad_h=ap.height,
                    pad_cx=ap.cx,
                    pad_cy=ap.cy,
                    rule=pkg.rule,
                    facing_dir=ap.facing_dir
                )
        else:
            ap.custom_slices = None
        return True

    def get_package_preview_data(self, pkg_name: str) -> Dict[str, Any]:
        target_name = self.package_aliases.get(pkg_name, pkg_name)
        if target_name not in self.packages:
            pkg = PackageTemplate(
                name=target_name, pin_count=2, pitch=1.702, pad_width=1.20, pad_height=1.30,
                pads_relative=[
                    {"pin": 1, "cx": -0.851, "cy": 0.0, "w": 1.20, "h": 1.30, "facing": "right"},
                    {"pin": 2, "cx": 0.851, "cy": 0.0, "w": 1.20, "h": 1.30, "facing": "left"}
                ]
            )
        else:
            pkg = self.packages[target_name]

        paste_elements = []
        for p in pkg.pads_relative:
            pin = p.get("pin", 1)
            facing = p.get("facing", "right")
            if pin in pkg.pad_overrides:
                pin_rule = self._resolve_pin_rule(pkg.rule, pkg.pad_overrides[pin])
                pw = float(pkg.pad_overrides[pin].get("w", p["w"]))
                ph = float(pkg.pad_overrides[pin].get("h", p["h"]))
                geom = self.generate_rule_aperture_geometry(
                    pad_w=pw, pad_h=ph,
                    pad_cx=p["cx"], pad_cy=p["cy"],
                    rule=pin_rule,
                    facing_dir=facing
                )
            else:
                geom = self.generate_rule_aperture_geometry(
                    pad_w=p["w"], pad_h=p["h"],
                    pad_cx=p["cx"], pad_cy=p["cy"],
                    rule=pkg.rule,
                    facing_dir=facing
                )
            if geom:
                paste_elements.extend(geom)

        return {
            "name": pkg.name,
            "pin_count": pkg.pin_count,
            "pitch": pkg.pitch,
            "pad_width": pkg.pad_width,
            "pad_height": pkg.pad_height,
            "category": pkg.category,
            "component_count": pkg.component_count,
            "outline": pkg.outline,
            "pads": pkg.pads_relative,
            "paste": paste_elements,
            "rule": pkg.rule.to_dict(),
            "pad_overrides": pkg.pad_overrides
        }

    # -------------------------------------------------------------
    # Kunden-Bibliotheken (Kunden-spezifische Regeln & Aliase)
    # -------------------------------------------------------------
    def list_customer_libraries(self, lib_dir: str = r"A:\Schablone\Libraries") -> List[str]:
        os.makedirs(lib_dir, exist_ok=True)
        libs = []
        for f in os.listdir(lib_dir):
            if f.lower().endswith(".json"):
                libs.append(os.path.splitext(f)[0])
        return sorted(libs) if libs else ["Standard"]

    def load_customer_library(self, customer_name: str, lib_dir: str = r"A:\Schablone\Libraries") -> bool:
        lib_file = os.path.join(lib_dir, f"{customer_name}.json")
        if not os.path.exists(lib_file):
            return False

        try:
            import json
            with open(lib_file, "r", encoding="utf-8") as f:
                data = json.load(f)

            self.current_customer = data.get("customer", customer_name)
            pkg_rules = data.get("packages", {})
            for pkg_k, rule_data in pkg_rules.items():
                if pkg_k in self.packages:
                    for rk, rv in rule_data.items():
                        if hasattr(self.packages[pkg_k].rule, rk):
                            setattr(self.packages[pkg_k].rule, rk, rv)

            aliases = data.get("aliases", {})
            self.package_aliases.update(aliases)
            return True
        except Exception as e:
            print(f"Fehler beim Laden der Kundenbibliothek {customer_name}: {e}")
            return False

    def save_customer_library(self, customer_name: str, lib_dir: str = r"A:\Schablone\Libraries") -> str:
        os.makedirs(lib_dir, exist_ok=True)
        lib_file = os.path.join(lib_dir, f"{customer_name}.json")

        import json
        payload = {
            "customer": customer_name,
            "version": "1.0",
            "packages": {name: pkg.rule.to_dict() for name, pkg in self.packages.items() if pkg.category != "Other"},
            "aliases": self.package_aliases
        }

        with open(lib_file, "w", encoding="utf-8") as f:
            json.dump(payload, f, indent=2, ensure_ascii=False)

        self.current_customer = customer_name
        return lib_file
