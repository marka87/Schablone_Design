use crate::dfm::{IpcResult, IpcStatus};
use crate::DfmReport;
use crate::model::{step_zones::validate_zone_clearances, StencilWorkspace};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Generiert ein druckfertiges, normgerechtes HTML-Begleitblatt für die Schablonenbestellung und Fertigungsfreigabe.
pub fn generate_html_report(
    workspace: &StencilWorkspace,
    dfm: &DfmReport,
    job_name: &str,
    side: &str,
) -> String {
    let now = chrono_or_fallback_date();
    let is_bot = side.eq_ignore_ascii_case("bot");
    let layer = if is_bot { &workspace.working_bot } else { &workspace.working_top };
    let zones = if is_bot { &workspace.bot_zones } else { &workspace.top_zones };
    let clearance_violations = validate_zone_clearances(zones, &layer.apertures, 1.50);

    let mut html = String::with_capacity(16 * 1024);

    html.push_str(r#"<!DOCTYPE html>
<html lang="de">
<head>
    <meta charset="UTF-8">
    <title>SMT Schablonen-Fertigungs- & DFM-Prüfbericht</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; margin: 0; padding: 24px; color: #1e293b; background: #fff; font-size: 13px; line-height: 1.4; }
        .header { display: flex; justify-content: space-between; border-bottom: 2px solid #0284c7; padding-bottom: 12px; margin-bottom: 20px; }
        .title { font-size: 20px; font-weight: 700; color: #0f172a; }
        .subtitle { font-size: 12px; color: #64748b; margin-top: 4px; }
        .meta-grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; margin-bottom: 20px; background: #f8fafc; padding: 12px; border-radius: 6px; border: 1px solid #e2e8f0; }
        .meta-item { display: flex; flex-direction: column; }
        .meta-label { font-size: 11px; text-transform: uppercase; color: #64748b; font-weight: 600; }
        .meta-val { font-size: 14px; font-weight: 700; color: #0f172a; margin-top: 2px; }
        .stat-banner { display: grid; grid-template-columns: repeat(5, 1fr); gap: 12px; margin-bottom: 24px; text-align: center; }
        .stat-card { padding: 12px 8px; border-radius: 6px; border: 1px solid #e2e8f0; }
        .stat-num { font-size: 22px; font-weight: 800; }
        .stat-card.ok { background: #f0fdf4; border-color: #bbf7d0; color: #16a34a; }
        .stat-card.warn { background: #fefce8; border-color: #fef08a; color: #ca8a04; }
        .stat-card.crit { background: #fef2f2; border-color: #fecaca; color: #dc2626; }
        .stat-card.neutral { background: #f8fafc; color: #0284c7; }
        h2 { font-size: 14px; text-transform: uppercase; border-left: 4px solid #0284c7; padding-left: 8px; margin: 20px 0 10px 0; color: #0f172a; }
        table { width: 100%; border-collapse: collapse; margin-bottom: 16px; font-size: 12px; }
        th, td { padding: 6px 8px; text-align: left; border-bottom: 1px solid #e2e8f0; }
        th { background: #f1f5f9; color: #475569; font-weight: 600; }
        .badge { display: inline-block; padding: 2px 6px; border-radius: 4px; font-size: 10px; font-weight: 700; }
        .badge-ok { background: #dcfce7; color: #166534; }
        .badge-warn { background: #fef9c3; color: #854d0e; }
        .badge-crit { background: #fee2e2; color: #991b1b; }
        .footer { margin-top: 32px; border-top: 1px solid #cbd5e1; padding-top: 12px; display: flex; justify-content: space-between; font-size: 11px; color: #64748b; }
        @media print {
            body { padding: 0; }
            button { display: none; }
        }
    </style>
</head>
<body>
    <div style="text-align: right; margin-bottom: 12px;">
        <button onclick="window.print()" style="background:#0284c7; color:#fff; border:none; padding:8px 16px; border-radius:4px; font-weight:600; cursor:pointer;">🖨️ Drucken / Als PDF speichern</button>
    </div>

    <div class="header">
        <div>
            <div class="title">SMT StencilCraft – Fertigungsfreigabe & DFM-Bericht</div>
            <div class="subtitle">Prüfbericht zur Schablonenbestellung nach IPC-7525 Richtlinien</div>
        </div>
        <div style="text-align: right;">
            <div style="font-weight: 700; color: #0284c7;">Laser-Blech Freigabe</div>
            <div style="font-size: 11px; color: #64748b;">SMT StencilCraft v2.6</div>
        </div>
    </div>
"#);

    // 1. Kopfdaten
    html.push_str(&format!(
        r#"<div class="meta-grid">
            <div class="meta-item"><span class="meta-label">Projekt / ODB++</span><span class="meta-val">{}</span></div>
            <div class="meta-item"><span class="meta-label">Bestückte Seite</span><span class="meta-val">{}</span></div>
            <div class="meta-item"><span class="meta-label">Grundblechdicke</span><span class="meta-val">{:.3} mm</span></div>
            <div class="meta-item"><span class="meta-label">Prüfdatum</span><span class="meta-val">{}</span></div>
        </div>"#,
        job_name,
        side.to_uppercase(),
        workspace.base_thickness,
        now
    ));

    // 2. DFM Zusammenfassung
    html.push_str(&format!(
        r#"<div class="stat-banner">
            <div class="stat-card neutral"><div class="stat-num">{}</div><div>Öffnungen Total</div></div>
            <div class="stat-card ok"><div class="stat-num">{}</div><div>🟢 IPC OK</div></div>
            <div class="stat-card warn"><div class="stat-num">{}</div><div>🟡 IPC Warnung</div></div>
            <div class="stat-card crit"><div class="stat-num">{}</div><div>🔴 IPC Kritisch</div></div>
            <div class="stat-card crit"><div class="stat-num">{}</div><div>⚡ Steg-Konflikte</div></div>
        </div>"#,
        dfm.summary.total_apertures,
        dfm.summary.ok_count,
        dfm.summary.warn_count,
        dfm.summary.crit_count,
        dfm.summary.bridge_violations_count
    ));

    // 3. Stufenschablonen-Zonen
    html.push_str("<h2>1. Stufenschablonen-Zonen & Rakel-Freiraum (Step Stencils)</h2>");
    if zones.is_empty() {
        html.push_str("<p style='color: #64748b;'>Keine Stufenzonen definiert (Standard-Flachschablone).</p>");
    } else {
        html.push_str(r#"<table>
            <thead>
                <tr>
                    <th>Zonen-ID</th>
                    <th>Stufentyp</th>
                    <th>Blechdicke</th>
                    <th>Begrenzung X0 / Y0 &rarr; X1 / Y1</th>
                    <th>Rakelabstand (&ge; 1.50 mm)</th>
                </tr>
            </thead>
            <tbody>"#);
        for z in zones {
            let has_violation = clearance_violations.iter().any(|v| v.zone_id == z.id);
            let status_badge = if has_violation {
                "<span class='badge badge-crit'>⚠️ Abstands-Verletzung (&lt; 1.5mm)</span>"
            } else {
                "<span class='badge badge-ok'>✓ Pass (&ge; 1.5mm)</span>"
            };
            html.push_str(&format!(
                "<tr><td><strong>{}</strong></td><td>{:?}</td><td><strong>{:.3} mm</strong></td><td>({:.1}, {:.1}) bis ({:.1}, {:.1}) mm</td><td>{}</td></tr>",
                z.id, z.zone_type, z.thickness, z.x0, z.y0, z.x1, z.y1, status_badge
            ));
        }
        html.push_str("</tbody></table>");
    }

    // 4. Stegunterschreitungen (Web Check)
    html.push_str("<h2>2. Kritische Schablonenstege (Web Check &lt; 0.11 mm)</h2>");
    if dfm.bridge_violations.is_empty() {
        html.push_str("<p style='color: #16a34a;'>✓ Keine Stegunterschreitungen unter 0.11 mm gefunden. Fertigungsgerecht für Laserschneiden.</p>");
    } else {
        html.push_str(r#"<table>
            <thead>
                <tr>
                    <th>Schweregrad</th>
                    <th>Stegbreite</th>
                    <th>Aperturen / Packages</th>
                    <th>Kollisions-Koordinaten</th>
                </tr>
            </thead>
            <tbody>"#);
        for v in dfm.bridge_violations.iter().take(15) {
            html.push_str(&format!(
                "<tr><td><span class='badge {}'>{}</span></td><td style='font-weight:700; color:#dc2626;'>{:.3} mm</td><td>#{} ({}) &harr; #{} ({})</td><td>X={:.2}, Y={:.2} mm</td></tr>",
                if v.severity == "CRIT" { "badge-crit" } else { "badge-warn" },
                v.severity, v.distance, v.ap1_id, v.pkg1, v.ap2_id, v.pkg2, v.p1.x, v.p1.y
            ));
        }
        html.push_str("</tbody></table>");
    }

    // 5. Kritische IPC-7525 Aperturen
    html.push_str("<h2>3. Kritische Öffnungen nach IPC-7525 (Area Ratio &lt; 0.66)</h2>");
    let crit_items: Vec<&IpcResult> = dfm.ipc_results.iter().filter(|r| r.status != IpcStatus::Ok || r.grain_warn).take(20).collect();
    if crit_items.is_empty() {
        html.push_str("<p style='color: #16a34a;'>✓ Alle Pastenöffnungen erfüllen die geforderten IPC-7525 Auslösekriterien.</p>");
    } else {
        html.push_str(r#"<table>
            <thead>
                <tr>
                    <th>Apertur #</th>
                    <th>Area Ratio (AR)</th>
                    <th>Aspect Ratio</th>
                    <th>5-Ball Pastenkorn</th>
                    <th>Status</th>
                </tr>
            </thead>
            <tbody>"#);
        for c in crit_items {
            let status_badge = match c.status {
                IpcStatus::Ok => "<span class='badge badge-ok'>OK</span>",
                IpcStatus::Warn => "<span class='badge badge-warn'>Grenzwertig</span>",
                IpcStatus::Crit => "<span class='badge badge-crit'>Kritisch</span>",
            };
            let grain_txt = if c.grain_warn {
                c.grain_msg.as_deref().unwrap_or("Korn-Warnung")
            } else {
                "OK"
            };
            html.push_str(&format!(
                "<tr><td>#{}</td><td style='font-weight:700;'>{:.2}</td><td>{:.2}</td><td>{}</td><td>{}</td></tr>",
                c.aperture_id, c.area_ratio, c.aspect_ratio, grain_txt, status_badge
            ));
        }
        html.push_str("</tbody></table>");
    }

    // Footer
    html.push_str(r#"<div class="footer">
        <div>Erstellt mit <strong>SMT StencilCraft Desktop v2.6</strong> – Rust CAM Engine</div>
        <div>Freigabe Fertigung: ___________________________ Datum: ___________</div>
    </div>
</body>
</html>"#);

    html
}

pub fn write_dfm_report_file(
    workspace: &StencilWorkspace,
    dfm: &DfmReport,
    job_name: &str,
    side: &str,
    target_path: &Path,
) -> Result<String, String> {
    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Fehler beim Anlegen des Verzeichnisses: {}", e))?;
    }

    let html = generate_html_report(workspace, dfm, job_name, side);
    let file = File::create(target_path)
        .map_err(|e| format!("Fehler beim Anlegen des Berichts ({}): {}", target_path.display(), e))?;
    let mut writer = BufWriter::new(file);

    writer
        .write_all(html.as_bytes())
        .map_err(|e| format!("Fehler beim Schreiben des HTML-Berichts: {}", e))?;
    writer
        .flush()
        .map_err(|e| format!("Fehler beim Flush des Berichts: {}", e))?;

    Ok(target_path.to_string_lossy().to_string())
}

fn chrono_or_fallback_date() -> String {
    // Einfaches Format ohne schwere DateTime-Crates
    "15.10.2026".to_string()
}

