use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PinGeometry {
    Round { diameter: f64 },
    Rect { width: f64, height: f64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipParams {
    pub d_drill: f64,
    pub pin_geometry: PinGeometry,
    pub pcb_thickness: f64,
    pub v_meniscus: f64,
    pub metal_ratio: f64,
    pub stencil_thickness: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipCross {
    pub core_size: f64,
    pub arm_width: f64,
    pub arm_length: f64,
    pub total_span: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipOverprintShapes {
    pub rect_w: f64,
    pub rect_h: f64,
    pub donut_inner_d: f64,
    pub donut_outer_d: f64,
    pub cross: PipCross,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipResult {
    pub v_hole_net_mm3: f64,
    pub v_paste_mm3: f64,
    pub a_aperture_mm2: f64,
    pub shapes: PipOverprintShapes,
}

/// Führt die physikalische Pin-in-Paste / Through-Hole Reflow Volumenberechnung aus.
/// Formeln:
///   V_Bohrung_Netto = ((pi * D_Bohrung^2) / 4 - A_Pin) * H_PCB
///   V_Paste = (V_Bohrung_Netto + V_Meniskus) / Metallanteil
///   Aperturflaeche = V_Paste / Blechdicke
pub fn calculate_pip(params: &PipParams) -> PipResult {
    let t_stencil = params.stencil_thickness.max(0.05);
    let metal = params.metal_ratio.clamp(0.20, 0.95);

    // 1. Pin-Querschnittsfläche A_Pin
    let a_pin = match params.pin_geometry {
        PinGeometry::Round { diameter } => (PI * diameter.powi(2)) / 4.0,
        PinGeometry::Rect { width, height } => width * height,
    };

    // 2. Netto-Bohrungsvolumen V_Bohrung_Netto
    let a_drill = (PI * params.d_drill.powi(2)) / 4.0;
    let v_hole_net = ((a_drill - a_pin).max(0.0)) * params.pcb_thickness.max(0.2);

    // 3. Meniskus-Volumen V_Meniskus (Default: 20% des Netto-Lochvolumens)
    let v_meniscus_eff = if params.v_meniscus <= 0.0 {
        0.20 * v_hole_net
    } else {
        params.v_meniscus
    };

    // 4. Erforderliches Nasspasten-Volumen
    let v_paste = (v_hole_net + v_meniscus_eff) / metal;

    // 5. Erforderliche Schablonenöffnungs-Fläche
    let a_aperture = v_paste / t_stencil;

    // 6. Overprint-Geometrien berechnen
    // a) Rechteck
    let rect_side = a_aperture.sqrt();

    // b) Donut / Annular Ring
    let donut_in = params.d_drill + 0.10; // Bohrungsfreistellung
    let donut_out = ((4.0 * a_aperture / PI) + donut_in.powi(2)).sqrt();

    // c) Kreuzform (Cross-Shape)
    let core = params.d_drill + 0.20;
    let arm_w = core * 0.70;
    let core_area = core * core;
    let rem_area = (a_aperture - core_area).max(0.0);
    let arm_l = rem_area / (4.0 * arm_w);
    let span = core + 2.0 * arm_l;

    PipResult {
        v_hole_net_mm3: v_hole_net,
        v_paste_mm3: v_paste,
        a_aperture_mm2: a_aperture,
        shapes: PipOverprintShapes {
            rect_w: rect_side,
            rect_h: rect_side,
            donut_inner_d: donut_in,
            donut_outer_d: donut_out,
            cross: PipCross {
                core_size: core,
                arm_width: arm_w,
                arm_length: arm_l,
                total_span: span,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pip_formula() {
        let params = PipParams {
            d_drill: 1.0,
            pin_geometry: PinGeometry::Round { diameter: 0.6 },
            pcb_thickness: 1.6,
            v_meniscus: 0.0,
            metal_ratio: 0.50,
            stencil_thickness: 0.12,
        };

        let res = calculate_pip(&params);
        assert!(res.v_hole_net_mm3 > 0.7 && res.v_hole_net_mm3 < 0.9);
        assert!(res.v_paste_mm3 > 1.8 && res.v_paste_mm3 < 2.2);
        assert!(res.a_aperture_mm2 > 15.0 && res.a_aperture_mm2 < 18.0);
    }
}

