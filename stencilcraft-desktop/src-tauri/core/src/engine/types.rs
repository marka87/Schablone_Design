//! Datenmodell der Stencil-Engine (1:1 zur Python `stencil_engine.py`, JSON-kompatibel zur Web-UI).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Punkt in mm. Wird als `[x, y]` serialisiert (wie Python-Tupel).
pub type Pt = (f64, f64);

#[inline]
pub fn r3(v: f64) -> f64 {
    (v * 1_000.0).round() / 1_000.0
}

#[inline]
pub fn r4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

#[inline]
pub fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[inline]
pub fn r5(v: f64) -> f64 {
    (v * 100_000.0).round() / 100_000.0
}

pub fn round_pts(pts: &[Pt]) -> Vec<Pt> {
    pts.iter().map(|&(x, y)| (r4(x), r4(y))).collect()
}

/// Ein Teilstück einer modifizierten Apertur (Slice, Polygon, Overprint ...).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Slice {
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pts: Option<Vec<Pt>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_inner: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_outer: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin: Option<i64>,
}

impl Slice {
    pub fn rect(cx: f64, cy: f64, w: f64, h: f64) -> Self {
        Slice { cx, cy, w, h, ..Default::default() }
    }

    pub fn polygon_pts(&self) -> Option<&Vec<Pt>> {
        self.pts.as_ref().filter(|p| p.len() >= 3)
    }

    pub fn is_circle(&self) -> bool {
        self.shape.as_deref() == Some("circle")
    }
}

/// Kupfer-/Maskenpad als Bounding-Box (für Korrelation und Fallback-Rendering).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadBox {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub round: bool,
}

impl PadBox {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        PadBox { x0, y0, x1, y1, w: x1 - x0, h: y1 - y0, round: false }
    }
}

#[derive(Debug, Clone)]
pub struct ApertureItem {
    pub id: usize,
    pub cx: f64,
    pub cy: f64,
    pub bbox: [f64; 4],
    pub shape_type: String, // circle | rectangle | polygon
    pub width: f64,
    pub height: f64,
    pub area: f64,
    pub perimeter: f64,
    pub points: Vec<Pt>,
    pub custom_slices: Option<Vec<Slice>>,
    pub package_group: String,
    pub package_name: String,
    pub component_id: String,
    pub pin_number: i64,
    pub facing_dir: String,
    pub is_manually_overridden: bool,
    pub pad_ref: Option<PadBox>,
    /// Kupferpad der Außenlage (ODB++), Basis der Regel "Package-Gruppe"
    pub copper_ref: Option<PadBox>,
    /// Aus dem Kupfer ergänzt (keine Pastenöffnung in der Kundenlage)
    pub from_copper: bool,
}

impl ApertureItem {
    pub fn new(id: usize, shape_type: &str, cx: f64, cy: f64, width: f64, height: f64) -> Self {
        let (area, perimeter) = if shape_type == "circle" {
            (std::f64::consts::PI * (width / 2.0).powi(2), std::f64::consts::PI * width)
        } else {
            (width * height, 2.0 * (width + height))
        };
        ApertureItem {
            id,
            cx,
            cy,
            bbox: [cx - width / 2.0, cy - height / 2.0, cx + width / 2.0, cy + height / 2.0],
            shape_type: shape_type.to_string(),
            width,
            height,
            area,
            perimeter,
            points: Vec::new(),
            custom_slices: None,
            package_group: format!("{:.2}x{:.2}", width, height),
            package_name: String::new(),
            component_id: String::new(),
            pin_number: 1,
            facing_dir: "right".into(),
            is_manually_overridden: false,
            pad_ref: None,
            copper_ref: None,
            from_copper: false,
        }
    }

    /// Polygon-Apertur aus Eckpunkten (Fläche/Umfang exakt nach Gauß).
    pub fn from_polygon(id: usize, pts: &[Pt]) -> Option<Self> {
        if pts.len() < 3 {
            return None;
        }
        let (min_x, min_y, max_x, max_y) = crate::engine::geometry::bounds(pts);
        let mut ap = ApertureItem::new(id, "polygon", (min_x + max_x) / 2.0, (min_y + max_y) / 2.0, max_x - min_x, max_y - min_y);
        ap.bbox = [min_x, min_y, max_x, max_y];
        ap.area = crate::engine::geometry::polygon_area(pts);
        ap.perimeter = crate::engine::geometry::polygon_perimeter(pts);
        ap.points = round_pts(pts);
        Some(ap)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentItem {
    pub refdes: String,
    pub package: String,
    pub cx: f64,
    pub cy: f64,
    pub width: f64,
    pub height: f64,
    #[serde(rename = "rot")]
    pub rotation: f64,
    pub side: String,
    pub outline: Vec<Pt>,
    /// Lage von Pin 1 (Board-Koordinaten) für die Markierung
    #[serde(default)]
    pub pin1: Option<Pt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<CompPin>,
}

/// Bauteil-Pin: Name, x, y, kleinste Padkante
pub type CompPin = (String, f64, f64, f64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StencilZone {
    pub id: String,
    pub name: String,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub thickness: f64,
    pub is_step_down: bool,
    /// Stufenzone (ändert die Blechdicke); false = reine Regelzone
    #[serde(default = "default_true")]
    pub step: bool,
    /// Zonenregel für alle Öffnungen, deren Mitte in der Zone liegt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<PackageRule>,
}

fn default_true() -> bool {
    true
}

impl StencilZone {
    pub fn width(&self) -> f64 {
        (self.x1 - self.x0).abs()
    }
    pub fn height(&self) -> f64 {
        (self.y1 - self.y0).abs()
    }
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let (min_x, max_x) = (self.x0.min(self.x1), self.x0.max(self.x1));
        let (min_y, max_y) = (self.y0.min(self.y1), self.y0.max(self.y1));
        min_x <= x && x <= max_x && min_y <= y && y <= max_y
    }
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "x0": r3(self.x0.min(self.x1)),
            "y0": r3(self.y0.min(self.y1)),
            "x1": r3(self.x0.max(self.x1)),
            "y1": r3(self.y0.max(self.y1)),
            "thickness": self.thickness,
            "is_step_down": self.is_step_down,
            "width": r3(self.width()),
            "height": r3(self.height()),
            "step": self.step,
            "rule": self.rule.as_ref().map(|r| r.to_json()),
        })
    }
}

/// Vektor-Primitive für das Canvas-Rendering der Lagen.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t")]
pub enum Prim {
    #[serde(rename = "r")]
    Rect { x: f64, y: f64, w: f64, h: f64 },
    #[serde(rename = "c")]
    Circle { x: f64, y: f64, r: f64 },
    #[serde(rename = "l")]
    Line { x1: f64, y1: f64, x2: f64, y2: f64, w: f64 },
    #[serde(rename = "p")]
    Poly { pts: Vec<Pt> },
    #[serde(rename = "comp")]
    Comp { refdes: String, pkg: String, x: f64, y: f64, w: f64, h: f64, rot: f64, o: Vec<Pt>, p1: Option<Pt>, pn: Vec<CompPin> },
}

impl Prim {
    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> Prim {
        Prim::Rect { x: r3(x), y: r3(y), w: r4(w), h: r4(h) }
    }
    pub fn circle(x: f64, y: f64, r: f64) -> Prim {
        Prim::Circle { x: r3(x), y: r3(y), r: r4(r) }
    }
    pub fn line(x1: f64, y1: f64, x2: f64, y2: f64, w: f64) -> Prim {
        Prim::Line { x1: r3(x1), y1: r3(y1), x2: r3(x2), y2: r3(y2), w: r3(w) }
    }
    pub fn poly(pts: &[Pt]) -> Prim {
        Prim::Poly { pts: pts.iter().map(|&(x, y)| (r3(x), r3(y))).collect() }
    }
}

#[derive(Debug, Clone)]
pub struct BoardLayer {
    pub id: String,
    pub name: String,
    pub filename: String,
    pub layer_type: String,
    pub side: String,
    pub color: String,
    pub visible: bool,
    pub opacity: f64,
    pub file_path: String,
    pub primitives: Vec<Prim>,
}

impl BoardLayer {
    pub fn summary(&self) -> Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "filename": self.filename,
            "type": self.layer_type,
            "side": self.side,
            "color": self.color,
            "visible": self.visible,
            "opacity": self.opacity,
            "count": self.primitives.len(),
            "file_path": self.file_path,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PackageRule {
    pub name: String,
    pub rule_type: String,
    pub undersize_w: f64,
    pub undersize_h: f64,
    pub corner_radius: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub offset_move_away: f64,
    pub house_cut_pct: f64,
    pub pitch_reduction_pct: f64,
    pub length_usage_pct: f64,
    pub bridge_count_x: i64,
    pub bridge_count_y: i64,
    pub bridge_width: f64,
    pub donut_inner_dia: f64,
    pub rotate_bridges_45: bool,
    pub try_find_direction: bool,
    pub toe_heel_shift: f64,
    pub chamfer_size: f64,
    pub chamfer_mode: String,
    pub pin_minus_mask: bool,
    /// Feste Öffnungsgröße (0 = aus Padgröße abgeleitet) – "Size Width/Height in MM"
    pub size_w: f64,
    pub size_h: f64,
    // Parameter der Regel "Package-Gruppe" (PCB Investigator: Handle Stencil Creation by Package Group)
    pub bga_threshold_square: f64,
    pub chip_threshold_houses: f64,
    pub invert_house_below: f64,
    pub mini_chip_reduction: f64,
    pub min_array_field: f64,
    pub distance_array_fields: f64,
    pub utilization_array_pct: f64,
    pub utilization_house_pct: f64,
    // Pin-in-Paste (THT-Reflow)
    pub pip_pcb_thickness: f64,
    pub pip_metal_pct: f64,
    pub pip_pin_clearance: f64,
    /// "circle", "rect" oder "donut"
    pub pip_shape: String,
    /// Größte Öffnung (0 = Pitch − Mindeststeg)
    pub pip_max_size: f64,
    // Länglich & Schmal
    /// Pastenabstand zu Öffnungsbreite quer zum Pitch (1.0 = Abstand gleich Öffnungsbreite)
    #[serde(default = "default_gap_ratio")]
    pub gap_ratio: f64,
    /// Längenausgleich: so viel % der Verschmälerung wird als Mehrlänge zugegeben
    #[serde(default = "default_length_comp")]
    pub length_comp_pct: f64,
    /// Versatz der Öffnung vom Bauteilzentrum nach außen (mm)
    #[serde(default = "default_outward_shift")]
    pub outward_shift: f64,
    /// Richtung für "nach außen" (Move away, Versatz, Toe-/Heel-Shift):
    /// "long" = entlang der Pad-Längsachse, "auto" = zum Bauteilzentrum, "x" / "y" = nur diese Package-Achse
    #[serde(default = "default_move_axis")]
    pub move_axis: String,
    /// Parameter aus PCB Investigator ohne eigene Entsprechung (für verlustfreien Rückexport)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

impl Default for PackageRule {
    fn default() -> Self {
        PackageRule {
            name: "Simple Rectangle Surface".into(),
            rule_type: "simple_rect".into(),
            undersize_w: 0.025,
            undersize_h: 0.025,
            corner_radius: 0.075,
            offset_x: 0.0,
            offset_y: 0.0,
            offset_move_away: 0.0,
            house_cut_pct: 35.0,
            pitch_reduction_pct: 20.0,
            length_usage_pct: 95.0,
            bridge_count_x: 0,
            bridge_count_y: 0,
            bridge_width: 0.30,
            donut_inner_dia: 0.0,
            rotate_bridges_45: false,
            try_find_direction: true,
            toe_heel_shift: 0.0,
            chamfer_size: 0.0,
            chamfer_mode: "all".into(),
            pin_minus_mask: false,
            size_w: 0.0,
            size_h: 0.0,
            bga_threshold_square: 0.41,
            chip_threshold_houses: 2.49,
            invert_house_below: 0.75,
            mini_chip_reduction: 0.075,
            min_array_field: 0.75,
            distance_array_fields: 0.25,
            utilization_array_pct: 40.0,
            utilization_house_pct: 0.0,
            pip_pcb_thickness: 1.6,
            pip_metal_pct: 50.0,
            pip_pin_clearance: 0.25,
            pip_shape: "circle".into(),
            pip_max_size: 0.0,
            gap_ratio: default_gap_ratio(),
            length_comp_pct: default_length_comp(),
            outward_shift: default_outward_shift(),
            move_axis: default_move_axis(),
            extra: BTreeMap::new(),
        }
    }
}

fn default_gap_ratio() -> f64 {
    1.0
}
fn default_length_comp() -> f64 {
    70.0
}
fn default_outward_shift() -> f64 {
    0.025
}
fn default_move_axis() -> String {
    "long".into()
}

pub fn val_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().replace(',', ".").parse().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn val_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|f| f != 0.0),
        Value::String(s) => Some(matches!(s.trim().to_lowercase().as_str(), "true" | "1" | "yes" | "on")),
        Value::Null => Some(false),
        _ => None,
    }
}

pub fn val_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

impl PackageRule {
    /// Übernimmt bekannte Felder aus einem JSON-Objekt (Pendant zu Python `setattr` für vorhandene Attribute).
    /// Werte werden typsicher konvertiert; ungültige Werte werden ignoriert.
    pub fn apply_map(&mut self, map: &Map<String, Value>) {
        for (k, v) in map {
            match k.as_str() {
                "name" => { if let Some(s) = val_str(v) { self.name = s } }
                "rule_type" => { if let Some(s) = val_str(v) { self.rule_type = s } }
                "chamfer_mode" => { if let Some(s) = val_str(v) { self.chamfer_mode = s } }
                "move_axis" => { if let Some(s) = val_str(v).filter(|s| matches!(s.as_str(), "long" | "auto" | "x" | "y")) { self.move_axis = s } }
                "undersize_w" => { if let Some(f) = val_f64(v) { self.undersize_w = f } }
                "undersize_h" => { if let Some(f) = val_f64(v) { self.undersize_h = f } }
                "corner_radius" => { if let Some(f) = val_f64(v) { self.corner_radius = f } }
                "offset_x" => { if let Some(f) = val_f64(v) { self.offset_x = f } }
                "offset_y" => { if let Some(f) = val_f64(v) { self.offset_y = f } }
                "offset_move_away" => { if let Some(f) = val_f64(v) { self.offset_move_away = f } }
                "house_cut_pct" => { if let Some(f) = val_f64(v) { self.house_cut_pct = f } }
                "pitch_reduction_pct" => { if let Some(f) = val_f64(v) { self.pitch_reduction_pct = f } }
                "length_usage_pct" => { if let Some(f) = val_f64(v) { self.length_usage_pct = f } }
                "bridge_count_x" => { if let Some(f) = val_f64(v) { self.bridge_count_x = f.round().max(0.0) as i64 } }
                "bridge_count_y" => { if let Some(f) = val_f64(v) { self.bridge_count_y = f.round().max(0.0) as i64 } }
                "bridge_width" => { if let Some(f) = val_f64(v) { self.bridge_width = f } }
                "donut_inner_dia" => { if let Some(f) = val_f64(v) { self.donut_inner_dia = f } }
                "toe_heel_shift" => { if let Some(f) = val_f64(v) { self.toe_heel_shift = f } }
                "chamfer_size" => { if let Some(f) = val_f64(v) { self.chamfer_size = f } }
                "size_w" => { if let Some(f) = val_f64(v) { self.size_w = f.max(0.0) } }
                "size_h" => { if let Some(f) = val_f64(v) { self.size_h = f.max(0.0) } }
                "bga_threshold_square" => { if let Some(f) = val_f64(v) { self.bga_threshold_square = f } }
                "chip_threshold_houses" => { if let Some(f) = val_f64(v) { self.chip_threshold_houses = f } }
                "invert_house_below" => { if let Some(f) = val_f64(v) { self.invert_house_below = f } }
                "mini_chip_reduction" => { if let Some(f) = val_f64(v) { self.mini_chip_reduction = f } }
                "min_array_field" => { if let Some(f) = val_f64(v) { self.min_array_field = f } }
                "distance_array_fields" => { if let Some(f) = val_f64(v) { self.distance_array_fields = f } }
                "utilization_array_pct" => { if let Some(f) = val_f64(v) { self.utilization_array_pct = f } }
                "utilization_house_pct" => { if let Some(f) = val_f64(v) { self.utilization_house_pct = f } }
                "pip_pcb_thickness" => { if let Some(f) = val_f64(v) { self.pip_pcb_thickness = f.max(0.1) } }
                "pip_metal_pct" => { if let Some(f) = val_f64(v) { self.pip_metal_pct = f.clamp(10.0, 100.0) } }
                "pip_pin_clearance" => { if let Some(f) = val_f64(v) { self.pip_pin_clearance = f.max(0.0) } }
                "pip_shape" => { if let Some(s) = val_str(v) { self.pip_shape = s } }
                "pip_max_size" => { if let Some(f) = val_f64(v) { self.pip_max_size = f.max(0.0) } }
                "gap_ratio" => { if let Some(f) = val_f64(v) { self.gap_ratio = f.clamp(0.2, 5.0) } }
                "length_comp_pct" => { if let Some(f) = val_f64(v) { self.length_comp_pct = f.clamp(0.0, 200.0) } }
                "outward_shift" => { if let Some(f) = val_f64(v) { self.outward_shift = f.clamp(-0.5, 0.5) } }
                "rotate_bridges_45" => { if let Some(b) = val_bool(v) { self.rotate_bridges_45 = b } }
                "try_find_direction" => { if let Some(b) = val_bool(v) { self.try_find_direction = b } }
                "pin_minus_mask" => { if let Some(b) = val_bool(v) { self.pin_minus_mask = b } }
                _ => {}
            }
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn with(rule_type: &str, name: &str) -> Self {
        PackageRule { rule_type: rule_type.into(), name: name.into(), ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadRel {
    pub pin: i64,
    pub cx: f64,
    pub cy: f64,
    pub w: f64,
    pub h: f64,
    pub facing: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_thermal: bool,
    /// Padform: "rect" oder "circle"
    #[serde(default = "rect_shape")]
    pub shape: String,
    /// Lötstopp-Freistellung (Package-Rahmen), falls bekannt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_w: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_round: Option<bool>,
    /// Bohrdurchmesser (THT), falls bekannt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill: Option<f64>,
    /// Kupferpad (B, H, rund) im Package-Rahmen, falls bekannt
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copper: Option<(f64, f64, bool)>,
}

fn rect_shape() -> String {
    "rect".into()
}

impl PadRel {
    pub fn new(pin: i64, cx: f64, cy: f64, w: f64, h: f64, facing: &str) -> Self {
        PadRel { pin, cx, cy, w, h, facing: facing.into(), is_thermal: false, shape: rect_shape(), mask_w: None, mask_h: None, mask_round: None, drill: None, copper: None }
    }

    pub fn round(mut self, round: bool) -> Self {
        self.shape = if round { "circle" } else { "rect" }.into();
        self
    }

    pub fn is_round(&self) -> bool {
        self.shape == "circle"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outline {
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageTemplate {
    pub name: String,
    pub pin_count: usize,
    pub pitch: f64,
    pub pad_width: f64,
    pub pad_height: f64,
    pub pads_relative: Vec<PadRel>,
    pub component_count: usize,
    pub rule: PackageRule,
    pub category: String,
    pub outline: Outline,
    pub pad_overrides: BTreeMap<i64, Map<String, Value>>,
    /// Baumgruppe wie PCB Investigator: "SMD", "THT", "Other", "Fiducial"
    #[serde(default = "default_group")]
    pub group: String,
}

fn default_group() -> String {
    "SMD".into()
}

impl PackageTemplate {
    pub fn new(name: &str, pin_count: usize, pitch: f64, pad_width: f64, pad_height: f64) -> Self {
        PackageTemplate {
            name: name.into(),
            pin_count,
            pitch,
            pad_width,
            pad_height,
            pads_relative: Vec::new(),
            component_count: 1,
            rule: PackageRule::default(),
            category: "SMD Passives".into(),
            outline: Outline { w: 2.0, h: 1.25 },
            pad_overrides: BTreeMap::new(),
            group: "SMD".into(),
        }
    }

    pub fn overrides_json(&self) -> Value {
        let mut m = Map::new();
        for (k, v) in &self.pad_overrides {
            m.insert(k.to_string(), Value::Object(v.clone()));
        }
        Value::Object(m)
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "pin_count": self.pin_count,
            "pitch": r3(self.pitch),
            "pad_width": r3(self.pad_width),
            "pad_height": r3(self.pad_height),
            "component_count": self.component_count,
            "category": self.category,
            "group": self.group,
            "outline": self.outline,
            "pads_relative": self.pads_relative,
            "rule": self.rule.to_json(),
            "pad_overrides": self.overrides_json(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BridgeViolation {
    pub id: usize,
    pub ap_id_1: usize,
    pub ap_id_2: usize,
    pub pkg_1: String,
    pub pkg_2: String,
    pub dist: f64,
    pub min_bridge: f64,
    pub severity: String,
    pub p1: Pt,
    pub p2: Pt,
    pub msg: String,
}

/// Ergebnis der IPC-7525-Bewertung einer Apertur.
#[derive(Debug, Clone, Serialize)]
pub struct IpcResult {
    pub effective_thickness: f64,
    pub zone_id: Option<String>,
    pub area_ratio: f64,
    pub aspect_ratio: f64,
    pub status: &'static str,
    pub clearance_warn: bool,
    pub min_ap_dim: f64,
    pub paste_type: String,
    pub grain_warn: bool,
    pub grain_status: &'static str,
    pub grain_msg: String,
    pub min_required_w: f64,
}
