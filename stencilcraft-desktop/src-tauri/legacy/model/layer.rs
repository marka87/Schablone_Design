use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn dist(&self, other: &Point) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PadShape {
    Rect,
    Round,
    Polygon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pad {
    pub id: usize,
    pub refdes: String,
    pub pin: usize,
    pub cx: f64,
    pub cy: f64,
    pub width: f64,
    pub height: f64,
    pub shape: PadShape,
    pub package_name: String,
    pub facing_dir: String, // "left", "right", "up", "down"
    pub is_thermal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleConfig {
    pub rule_type: String, // "simple_rect", "house_builder", "inverted_house_builder", "array_slicing", "elongated_pitch", "chamfer"
    pub undersize_w: f64,
    pub undersize_h: f64,
    pub house_cut_pct: f64,
    pub pitch_reduction_pct: f64,
    pub bridge_nx: usize,
    pub bridge_my: usize,
    pub bridge_width: f64,
    pub rotate_45: bool,
    pub corner_radius: f64,
    pub chamfer_size: f64,
    pub chamfer_mode: String,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            rule_type: "simple_rect".to_string(),
            undersize_w: 0.025,
            undersize_h: 0.025,
            house_cut_pct: 10.0,
            pitch_reduction_pct: 15.0,
            bridge_nx: 2,
            bridge_my: 2,
            bridge_width: 0.20,
            rotate_45: false,
            corner_radius: 0.0,
            chamfer_size: 0.0,
            chamfer_mode: "all".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Aperture {
    pub id: usize,
    pub pad_id: Option<usize>,
    pub package_name: String,
    pub component_refdes: String,
    pub pin: usize,
    pub cx: f64,
    pub cy: f64,
    pub width: f64,
    pub height: f64,
    pub facing_dir: String,
    pub rotation: f64,
    pub points: Vec<Point>,
    pub area: f64,
    pub shape_type: String,
    pub is_modified: bool,
    pub is_custom: bool,
    pub manual_override: bool,
}

impl Aperture {
    pub fn from_pad(pad: &Pad) -> Self {
        let half_w = pad.width / 2.0;
        let half_h = pad.height / 2.0;
        let points = vec![
            Point::new(pad.cx - half_w, pad.cy - half_h),
            Point::new(pad.cx + half_w, pad.cy - half_h),
            Point::new(pad.cx + half_w, pad.cy + half_h),
            Point::new(pad.cx - half_w, pad.cy + half_h),
        ];
        let area = pad.width * pad.height;

        Self {
            id: pad.id,
            pad_id: Some(pad.id),
            package_name: pad.package_name.clone(),
            component_refdes: pad.refdes.clone(),
            pin: pad.pin,
            cx: pad.cx,
            cy: pad.cy,
            width: pad.width,
            height: pad.height,
            facing_dir: pad.facing_dir.clone(),
            rotation: 0.0,
            points,
            area,
            shape_type: "rectangle".to_string(),
            is_modified: false,
            is_custom: false,
            manual_override: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub refdes: String,
    pub package_name: String,
    pub cx: f64,
    pub cy: f64,
    pub rotation: f64,
    pub side: String, // "TOP" | "BOT"
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerType {
    OriginalPaste,
    StencilWorking,
    Copper,
    SolderMask,
    ComponentLayer,
    Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub layer_type: LayerType,
    pub side: String, // "TOP" | "BOT"
    pub is_readonly: bool,
    pub visible: bool,
    pub color: String,
    pub opacity: f32,
    pub pads: Vec<Pad>,
    pub apertures: Vec<Aperture>,
}

impl Layer {
    pub fn new(id: &str, name: &str, layer_type: LayerType, side: &str, is_readonly: bool, color: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            layer_type,
            side: side.to_uppercase(),
            is_readonly,
            visible: true,
            color: color.to_string(),
            opacity: 1.0,
            pads: Vec::new(),
            apertures: Vec::new(),
        }
    }
}

use crate::model::step_zones::StepZone;

/// Verwaltet die strikte Trennung zwischen geschütztem Original und modifizierbarem Schablonen-Zwischenspeicher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StencilWorkspace {
    pub base_thickness: f64,
    pub original_top: Layer,
    pub original_bot: Layer,
    pub working_top: Layer,
    pub working_bot: Layer,
    pub top_zones: Vec<StepZone>,
    pub bot_zones: Vec<StepZone>,
    pub components: Vec<Component>,
}

impl Default for StencilWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

impl StencilWorkspace {
    pub fn new() -> Self {
        Self {
            base_thickness: 0.12,
            original_top: Layer::new("orig_top", "TOP_PASTE (Kunde / Original)", LayerType::OriginalPaste, "TOP", true, "#e879f9"),
            original_bot: Layer::new("orig_bot", "BOT_PASTE (Kunde / Original)", LayerType::OriginalPaste, "BOT", true, "#c084fc"),
            working_top: Layer::new("work_top", "Schablone TOP (Zwischenspeicher)", LayerType::StencilWorking, "TOP", false, "#38bdf8"),
            working_bot: Layer::new("work_bot", "Schablone BOT (Zwischenspeicher)", LayerType::StencilWorking, "BOT", false, "#818cf8"),
            top_zones: Vec::new(),
            bot_zones: Vec::new(),
            components: Vec::new(),
        }
    }

    /// Lädt Pads und Aperturen in das geschützte Original und spiegelt sie in den Arbeits-Zwischenspeicher.
    pub fn load_dataset(&mut self, side: &str, pads: Vec<Pad>, components: Vec<Component>) {
        let is_top = side.eq_ignore_ascii_case("top");
        let orig = if is_top { &mut self.original_top } else { &mut self.original_bot };
        
        orig.pads = pads.clone();
        orig.apertures = pads.iter().map(Aperture::from_pad).collect();

        let work = if is_top { &mut self.working_top } else { &mut self.working_bot };
        work.pads = pads;
        work.apertures = orig.apertures.clone();
        work.is_readonly = false;

        self.components = components;
    }

    /// Setzt den Arbeits-Zwischenspeicher auf die Original-Kundendaten zurück (non-destruktiv).
    pub fn reset_working(&mut self, side: &str) {
        if side.eq_ignore_ascii_case("top") {
            self.working_top.apertures = self.original_top.apertures.clone();
        } else {
            self.working_bot.apertures = self.original_bot.apertures.clone();
        }
    }

    pub fn get_working_layer(&self, side: &str) -> &Layer {
        if side.eq_ignore_ascii_case("top") {
            &self.working_top
        } else {
            &self.working_bot
        }
    }

    pub fn get_working_layer_mut(&mut self, side: &str) -> &mut Layer {
        if side.eq_ignore_ascii_case("top") {
            &mut self.working_top
        } else {
            &mut self.working_bot
        }
    }
}

