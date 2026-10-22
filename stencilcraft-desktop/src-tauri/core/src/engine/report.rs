//! Prüfbericht (HTML, druckbar bzw. als PDF speicherbar) als Begleitblatt zur Schablonenbestellung.

use super::export::today;
use super::types::*;
use super::StencilEngine;
use std::collections::BTreeMap;
use std::fmt::Write as _;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn status_color(s: &str) -> &'static str {
    match s {
        "OK" => "#16a34a",
        "WARN" => "#d97706",
        _ => "#dc2626",
    }
}

/// Kurzbeschreibung einer Regel mit den maßgeblichen Parametern.
fn rule_summary(r: &PackageRule) -> String {
    let base = format!("Undersize {:.3} · R {:.3}", r.undersize_w, r.corner_radius);
    match r.rule_type.as_str() {
        "package_group" => format!("Package-Gruppe · {base} · Array ≥{:.2} mm / {:.0} %", r.min_array_field, r.utilization_array_pct),
        "simple_rect" if r.size_w > 0.0 || r.size_h > 0.0 => format!("Rechteck fest {:.3}×{:.3} mm", r.size_w, r.size_h),
        "simple_rect" => format!("Rechteck · {base}"),
        "house_builder" => format!("HouseBuilder · {base} · Cut {:.0} %", r.house_cut_pct),
        "inverted_house_builder" => format!("Inverted House · {base} · Cut {:.0} %", r.house_cut_pct),
        "elongated_pitch" => format!("Länglich & Schmal · −{:.0} % Breite · {:.0} % Länge", r.pitch_reduction_pct, r.length_usage_pct),
        "array_slicing" => format!("Array {}×{} · Steg {:.2} mm{}", r.bridge_count_x + 1, r.bridge_count_y + 1, r.bridge_width, if r.rotate_bridges_45 { " · 45°" } else { "" }),
        "donut" => format!("Donut · Innen {:.2} mm · Steg {:.2} mm", r.donut_inner_dia, r.bridge_width),
        "take_existing" => "Original übernommen".into(),
        "ignore" => "keine Öffnung".into(),
        other => other.to_string(),
    }
}

impl StencilEngine {
    /// Zeichnung der aktiven Seite als SVG (Öffnungen nach IPC-Status eingefärbt).
    fn side_svg(&self, ipc: &[IpcResult]) -> String {
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        let mut grow = |x: f64, y: f64| {
            b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
        };
        for a in &self.apertures {
            grow(a.bbox[0], a.bbox[1]);
            grow(a.bbox[2], a.bbox[3]);
        }
        for s in &self.profile_paths {
            grow(s[0].0, s[0].1);
            grow(s[1].0, s[1].1);
        }
        if !b[0].is_finite() {
            return String::new();
        }
        let m = 2.0;
        let (x0, y1) = (b[0] - m, b[3] + m);
        let (ww, hh) = (b[2] - b[0] + 2.0 * m, b[3] - b[1] + 2.0 * m);
        let width = 1000.0;
        let sc = width / ww;
        let height = hh * sc;
        let tx = |x: f64| (x - x0) * sc;
        let ty = |y: f64| (y1 - y) * sc;
        let poly = |out: &mut String, pts: &[Pt], fill: &str| {
            let _ = write!(out, "<polygon fill=\"{fill}\" points=\"");
            for p in pts {
                let _ = write!(out, "{:.1},{:.1} ", tx(p.0), ty(p.1));
            }
            out.push_str("\"/>");
        };

        let mut out = String::new();
        let _ = write!(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {width:.0} {height:.0}\" class=\"board\">");
        let _ = write!(out, "<rect width=\"100%\" height=\"100%\" fill=\"#f8fafc\"/>");
        if !self.profile_paths.is_empty() {
            out.push_str("<path stroke=\"#64748b\" stroke-width=\"1\" fill=\"none\" d=\"");
            for s in &self.profile_paths {
                let _ = write!(out, "M{:.1} {:.1}L{:.1} {:.1}", tx(s[0].0), ty(s[0].1), tx(s[1].0), ty(s[1].1));
            }
            out.push_str("\"/>");
        }
        for z in &self.zones {
            let (zx0, zy0, zx1, zy1) = (z.x0.min(z.x1), z.y0.min(z.y1), z.x0.max(z.x1), z.y0.max(z.y1));
            let col = if z.is_step_down { "#7c3aed" } else { "#e11d48" };
            let _ = write!(
                out,
                "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"{col}\" fill-opacity=\"0.08\" stroke=\"{col}\" stroke-dasharray=\"6 4\"/>",
                tx(zx0), ty(zy1), (zx1 - zx0) * sc, (zy1 - zy0) * sc
            );
        }
        for (a, r) in self.apertures.iter().zip(ipc) {
            let fill = status_color(r.status);
            match a.custom_slices.as_ref() {
                Some(slices) => {
                    for s in slices {
                        if let Some(p) = s.polygon_pts() {
                            poly(&mut out, p, fill);
                        } else {
                            poly(&mut out, &super::geometry::rect_pts(s.cx, s.cy, s.w, s.h), fill);
                        }
                    }
                }
                None if a.shape_type == "circle" => {
                    let _ = write!(out, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{:.2}\" fill=\"{fill}\"/>", tx(a.cx), ty(a.cy), (a.width / 2.0 * sc).max(0.4));
                }
                None if a.points.len() >= 3 => poly(&mut out, &a.points, fill),
                None => poly(&mut out, &super::geometry::rect_pts(a.cx, a.cy, a.width, a.height), fill),
            }
        }
        for v in &self.bridge_violations {
            let (mx, my) = ((v.p1.0 + v.p2.0) / 2.0, (v.p1.1 + v.p2.1) / 2.0);
            let _ = write!(out, "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"5\" fill=\"none\" stroke=\"#dc2626\" stroke-width=\"1.5\"/>", tx(mx), ty(my));
        }
        out.push_str("</svg>");
        out
    }

    /// Erzeugt den Prüfbericht für die angegebenen Seiten.
    pub fn report_html(&mut self, sides: &[String]) -> String {
        let entry_side = self.current_side.clone();
        let mut h = String::with_capacity(256 * 1024);
        let job = self.job_name();
        let _ = write!(
            h,
            r#"<!DOCTYPE html><html lang="de"><head><meta charset="utf-8"><title>Prüfbericht {job}</title><style>
@page {{ size: A4; margin: 14mm; }}
body {{ font-family: "Segoe UI", Arial, sans-serif; color: #0f172a; margin: 24px; font-size: 12.5px; }}
h1 {{ font-size: 20px; margin: 0 0 4px; }} h2 {{ font-size: 16px; margin: 26px 0 8px; border-bottom: 2px solid #0284c7; padding-bottom: 4px; }}
h3 {{ font-size: 13px; margin: 16px 0 6px; }}
.sub {{ color: #64748b; margin-bottom: 14px; }}
.grid {{ display: grid; grid-template-columns: repeat(4, 1fr); gap: 8px; margin: 10px 0; }}
.kv {{ background: #f1f5f9; border: 1px solid #e2e8f0; border-radius: 6px; padding: 6px 8px; }}
.kv b {{ display: block; font-size: 10px; text-transform: uppercase; color: #64748b; }}
.kv span {{ font-size: 14px; font-weight: 700; }}
.ok {{ color: #16a34a; }} .warn {{ color: #d97706; }} .crit {{ color: #dc2626; }}
table {{ width: 100%; border-collapse: collapse; margin: 6px 0 12px; font-size: 11.5px; }}
th, td {{ border: 1px solid #e2e8f0; padding: 4px 6px; text-align: left; }} th {{ background: #f1f5f9; }}
td.num {{ text-align: right; font-family: Consolas, monospace; }}
svg.board {{ width: 100%; height: auto; border: 1px solid #cbd5e1; border-radius: 6px; margin: 6px 0; }}
.legend span {{ margin-right: 14px; }} .dot {{ display: inline-block; width: 10px; height: 10px; border-radius: 2px; margin-right: 4px; vertical-align: middle; }}
.side {{ page-break-before: always; }} .side:first-of-type {{ page-break-before: auto; }}
.foot {{ margin-top: 30px; color: #94a3b8; font-size: 10.5px; }}
@media print {{ .noprint {{ display: none; }} body {{ margin: 0; }} }}
</style></head><body>
<button class="noprint" onclick="window.print()" style="float:right;padding:6px 12px">Drucken / als PDF speichern</button>
<h1>Schablonen-Prüfbericht – {job}</h1>
<div class="sub">{dataset} · erstellt {date}{project}</div>
<div class="grid">
<div class="kv"><b>Pastentyp</b><span>{paste}</span></div>
<div class="kv"><b>IPC-Kriterien</b><span>{profile}: AR ≥ {ar:.2} · AspR ≥ {asp:.2}</span></div>
<div class="kv"><b>Mindeststeg</b><span>{bridge:.2} mm</span></div>
<div class="kv"><b>Kundenbibliothek</b><span>{cust}</span></div>
</div>"#,
            job = esc(&job),
            dataset = esc(&self.dataset_name),
            date = today(),
            project = self.project_path.as_ref().map(|p| format!(" · Projekt {}", esc(&p.to_string_lossy()))).unwrap_or_default(),
            paste = esc(&self.paste_type.to_uppercase()),
            profile = esc(&self.ipc_profile_label()),
            ar = self.ipc_min_ar,
            asp = self.ipc_min_aspect,
            bridge = self.min_bridge_width,
            cust = esc(&self.current_customer),
        );

        for side in sides {
            self.switch_side(side);
            if self.apertures.is_empty() {
                continue;
            }
            self.check_stencil_bridges(self.min_bridge_width);
            let ipc = self.all_ipc();
            let (mut ok, mut warn, mut crit, mut grain) = (0, 0, 0, 0);
            let mut min_ar = f64::INFINITY;
            for r in &ipc {
                match r.status {
                    "OK" => ok += 1,
                    "WARN" => warn += 1,
                    _ => crit += 1,
                }
                if r.grain_warn {
                    grain += 1;
                }
                min_ar = min_ar.min(r.area_ratio);
            }
            let modified = self.apertures.iter().filter(|a| a.custom_slices.is_some()).count();
            let _ = write!(
                h,
                r#"<section class="side"><h2>Seite {side} – Schablone {thick:.0} µm</h2>
<div class="grid">
<div class="kv"><b>Öffnungen</b><span>{n}</span></div>
<div class="kv"><b>Bearbeitet (Regel/manuell)</b><span>{modified}</span></div>
<div class="kv"><b>IPC OK / Warnung / Kritisch</b><span><span class="ok">{ok}</span> / <span class="warn">{warn}</span> / <span class="crit">{crit}</span></span></div>
<div class="kv"><b>Kleinstes Area Ratio</b><span>{min_ar:.3}</span></div>
<div class="kv"><b>5-Ball-Regel verletzt</b><span>{grain}</span></div>
<div class="kv"><b>Stegverletzungen</b><span class="{bcls}">{bridges}</span></div>
<div class="kv"><b>Stufenzonen</b><span>{zones}</span></div>
<div class="kv"><b>Rakelrichtung</b><span>{sq}</span></div>
</div>
<div class="legend"><span><i class="dot" style="background:#16a34a"></i>IPC OK</span><span><i class="dot" style="background:#d97706"></i>Warnung</span><span><i class="dot" style="background:#dc2626"></i>Kritisch</span><span>○ rot = Stegverletzung</span><span>gestrichelt = Stufenzone</span></div>
"#,
                side = side.to_uppercase(),
                thick = self.base_thickness * 1000.0,
                n = self.apertures.len(),
                min_ar = if min_ar.is_finite() { min_ar } else { 0.0 },
                bridges = self.bridge_violations.len(),
                bcls = if self.bridge_violations.is_empty() { "ok" } else { "crit" },
                zones = self.zones.iter().filter(|z| z.step).count(),
                sq = esc(&self.squeegee_dir),
            );
            h.push_str(&self.side_svg(&ipc));

            if !self.zones.is_empty() {
                h.push_str("<h3>Stufenzonen</h3><table><tr><th>Zone</th><th>Dicke</th><th>Art</th><th>Bereich (mm)</th></tr>");
                for z in &self.zones {
                    let _ = write!(
                        h,
                        "<tr><td>{}</td><td class=\"num\">{:.0} µm</td><td>{}</td><td class=\"num\">{:.2}, {:.2} – {:.2}, {:.2}</td></tr>",
                        esc(&z.name), z.thickness * 1000.0, if !z.step { "Regelzone (keine Stufe)" } else if z.is_step_down { "Step-Down" } else { "Step-Up" },
                        z.x0.min(z.x1), z.y0.min(z.y1), z.x0.max(z.x1), z.y0.max(z.y1)
                    );
                }
                h.push_str("</table>");
            }

            // Packages mit Regeln
            let mut per_pkg: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            for a in &self.apertures {
                let name = self.resolve_alias(&a.package_name);
                let e = per_pkg.entry(name).or_default();
                e.0 += 1;
                if a.custom_slices.is_some() {
                    e.1 += 1;
                }
            }
            h.push_str("<h3>Regeln je Package</h3><table><tr><th>Package</th><th>Gruppe</th><th>Regel</th><th>Pin-Anpassungen</th><th>Pads</th><th>bearbeitet</th></tr>");
            for (name, (n, m)) in &per_pkg {
                let (group, rule, ovs) = self
                    .packages
                    .get(name)
                    .map(|p| (p.group.clone(), rule_summary(&p.rule), p.pad_overrides.len()))
                    .unwrap_or_else(|| ("-".into(), "keine Regel (Original)".into(), 0));
                let _ = write!(h, "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{n}</td><td class=\"num\">{m}</td></tr>", esc(name), esc(&group), esc(&rule), ovs);
            }
            h.push_str("</table>");

            // Kritische/grenzwertige Öffnungen
            let mut issues: Vec<(&ApertureItem, &IpcResult)> = self.apertures.iter().zip(&ipc).filter(|(_, r)| r.status != "OK" || r.clearance_warn).collect();
            issues.sort_by(|a, b| a.1.area_ratio.partial_cmp(&b.1.area_ratio).unwrap_or(std::cmp::Ordering::Equal));
            if !issues.is_empty() {
                let _ = write!(h, "<h3>Auffällige Öffnungen ({} – die 60 kritischsten)</h3>", issues.len());
                h.push_str("<table><tr><th>#</th><th>Bauteil</th><th>Package</th><th>Pin</th><th>Größe (mm)</th><th>AR</th><th>AspR</th><th>Status</th><th>Hinweis</th></tr>");
                for (a, r) in issues.iter().take(60) {
                    let note = if r.grain_warn { r.grain_msg.clone() } else if r.clearance_warn { "zu nah an Stufenkante".into() } else { String::new() };
                    let _ = write!(
                        h,
                        "<tr><td class=\"num\">{}</td><td>{}</td><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{:.3}×{:.3}</td><td class=\"num\">{:.3}</td><td class=\"num\">{:.2}</td><td style=\"color:{}\">{}</td><td>{}</td></tr>",
                        a.id, esc(&a.component_id), esc(&a.package_name), a.pin_number, a.width, a.height, r.area_ratio, r.aspect_ratio,
                        status_color(r.status), r.status, esc(&note)
                    );
                }
                h.push_str("</table>");
            }

            if !self.bridge_violations.is_empty() {
                let _ = write!(h, "<h3>Stegverletzungen ({} – die 60 engsten)</h3>", self.bridge_violations.len());
                h.push_str("<table><tr><th>Öffnungen</th><th>Packages</th><th>Abstand</th><th>Grenze</th><th>Position (mm)</th></tr>");
                for v in self.bridge_violations.iter().take(60) {
                    let _ = write!(
                        h,
                        "<tr><td class=\"num\">{} ↔ {}</td><td>{} / {}</td><td class=\"num\" style=\"color:{}\">{:.3}</td><td class=\"num\">{:.3}</td><td class=\"num\">{:.2}, {:.2}</td></tr>",
                        v.ap_id_1, v.ap_id_2, esc(&v.pkg_1), esc(&v.pkg_2), status_color(&v.severity), v.dist, v.min_bridge, v.p1.0, v.p1.1
                    );
                }
                h.push_str("</table>");
            }
            h.push_str("</section>");
        }
        self.switch_side(&entry_side);
        let _ = write!(h, "<div class=\"foot\">Erstellt mit SMT StencilCraft Desktop 2.6 · {}</div></body></html>", today());
        h
    }
}
