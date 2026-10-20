//! Vergleich StencilCraft ⇄ PCB Investigator an einem Referenzjob.
//! Aufruf: cargo run --example pcbi_compare -- <job.tgz> <ruleTree.xml> <pcbi_top.ger> <pcbi_bot.ger>
use stencil_core::engine::{gerber, StencilEngine};
use std::collections::BTreeMap;

fn bbox(aps: &[stencil_core::engine::types::ApertureItem]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for a in aps {
        b = [b[0].min(a.cx), b[1].min(a.cy), b[2].max(a.cx), b[3].max(a.cy)];
    }
    b.map(|v| (v * 100.0).round() / 100.0)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tmp = std::env::temp_dir().join("sc_pcbi_compare");
    let _ = std::fs::create_dir_all(&tmp);
    let mut e = StencilEngine::new(0.12, tmp.clone());
    e.import_files_or_zip(&[a[0].clone()], "top", true).expect("Import");
    println!("Datensatz {} – Packages {}", e.dataset_name, e.packages.len());
    for (side, ger) in [("top", &a[2]), ("bot", &a[3])] {
        if side == "bot" {
            e.switch_side("bot");
        }
        let (rep, _) = e.import_rule_tree_file(std::path::Path::new(&a[1]), false).expect("XML");
        if side == "top" {
            println!("Bibliothek: {} Packages, übernommen {:?}", rep["library_packages"], rep["matched"].as_array().map(|v| v.len()));
            println!("  nicht im Datensatz: {}", rep["not_in_dataset"]);
            println!("  Pin-Regeln {}, Gruppen {}, unbekannt {}, Warnungen {}", rep["pin_rules"], rep["group_rules"], rep["unknown_rules"], rep["warnings"]);
            println!("  ohne Bibliothekseintrag: {}", rep["missing_in_library"].as_array().map_or(0, |v| v.len()));
        }
        let added = if std::env::var("NOADD").is_ok() { 0 } else { e.add_missing_paste() };
        println!("[{side}] aus Kupfer ergänzt: {added}");
        let n = e.generate_all();
        let out = tmp.join(format!("ours_{side}.gbr"));
        e.export_modified_gerber(&out).unwrap();
        let ours = gerber::parse_gerber_file(&out).unwrap().paste_apertures();
        let theirs = gerber::parse_gerber_file(std::path::Path::new(ger)).unwrap().paste_apertures();
        println!("\n[{side}] neu erzeugt {n} | Öffnungen StencilCraft {} / PCB-I {}", ours.len(), theirs.len());
        println!("  Board-Grenzen (Profil) {:?}", e.bounds.map(|v| (v * 100.0).round() / 100.0));
        if side == "bot" {
            for a in e.apertures.iter().filter(|a| a.cx < -10.0).take(2) {
                println!("  außerhalb: {} {} @({:.2},{:.2})", a.package_name, a.component_id, a.cx, a.cy);
            }
            for c in e.components.iter().filter(|c| c.cx < -10.0).take(3) {
                println!("  Bauteil außerhalb: {} {} @({:.2},{:.2}) side {}", c.refdes, c.package, c.cx, c.cy, c.side);
            }
        }
        println!("  bbox ours {:?} pcbi {:?}", bbox(&ours), bbox(&theirs));
        // Zuordnung: nächstes Zentrum ≤ 0.15 mm
        let mut used = vec![false; ours.len()];
        let mut miss_pkg: BTreeMap<String, usize> = BTreeMap::new();
        let mut diff_pkg: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
        let (mut matched, mut same) = (0, 0);
        let mut examples = 0;
        let pkg_at = |x: f64, y: f64| -> String {
            e.apertures.iter().min_by(|p, q| (p.cx - x).hypot(p.cy - y).total_cmp(&(q.cx - x).hypot(q.cy - y)))
                .map(|p| if p.package_name.is_empty() { "?".into() } else { p.package_name.clone() }).unwrap_or_default()
        };
        for t in &theirs {
            let best = ours.iter().enumerate().filter(|(i, _)| !used[*i]).map(|(i, o)| (i, (o.cx - t.cx).hypot(o.cy - t.cy))).min_by(|x, y| x.1.total_cmp(&y.1));
            match best.filter(|b| b.1 <= 0.15) {
                Some((i, _)) => {
                    used[i] = true;
                    matched += 1;
                    let o = &ours[i];
                    let rel = (o.area - t.area) / t.area.max(1e-6);
                    if rel.abs() <= 0.05 {
                        same += 1;
                    } else {
                        if std::env::var("EX").is_ok() && examples < 12 {
                            examples += 1;
                            let ap = e.apertures.iter().min_by(|p, q| (p.cx - t.cx).hypot(p.cy - t.cy).total_cmp(&(q.cx - t.cx).hypot(q.cy - t.cy))).unwrap();
                            println!("    EX {} pin {} paste {:.3}x{:.3} {} cu {:?} | ours {} {:.3}x{:.3} a{:.4} | pcbi {} {:.3}x{:.3} a{:.4} @({:.3},{:.3})", ap.package_name, ap.pin_number, ap.width, ap.height, ap.shape_type, ap.copper_ref.as_ref().map(|c| (c.w, c.h, c.round)), o.shape_type, o.width, o.height, o.area, t.shape_type, t.width, t.height, t.area, t.cx, t.cy);
                        }
                        let d = diff_pkg.entry(pkg_at(t.cx, t.cy)).or_default();
                        d.0 += 1;
                        d.1 += o.area;
                        d.2 += t.area;
                    }
                }
                None => {
                    if std::env::var("MISS").is_ok() && examples < 40 {
                        examples += 1;
                        let ap = e.apertures.iter().min_by(|p, q| (p.cx - t.cx).hypot(p.cy - t.cy).total_cmp(&(q.cx - t.cx).hypot(q.cy - t.cy))).unwrap();
                        let o = ours.iter().min_by(|p, q| (p.cx - t.cx).hypot(p.cy - t.cy).total_cmp(&(q.cx - t.cx).hypot(q.cy - t.cy))).unwrap();
                        println!("    MISS pcbi {:.3}x{:.3} a{:.4} @({:.3},{:.3}) | ap {} {} pin {} {:.3}x{:.3} @({:.3},{:.3}) facing {} | ours nearest {:.3}x{:.3} @({:.3},{:.3})", t.width, t.height, t.area, t.cx, t.cy, ap.package_name, ap.component_id, ap.pin_number, ap.width, ap.height, ap.cx, ap.cy, ap.facing_dir, o.width, o.height, o.cx, o.cy);
                    }
                    *miss_pkg.entry(pkg_at(t.cx, t.cy)).or_default() += 1
                }
            }
        }
        let mut extra: BTreeMap<String, usize> = BTreeMap::new();
        for (i, o) in ours.iter().enumerate() {
            if !used[i] {
                *extra.entry(pkg_at(o.cx, o.cy)).or_default() += 1;
            }
        }
        println!("  zugeordnet {matched}, davon Fläche ±5 %: {same}");
        println!("  nur PCB-I: {:?}", miss_pkg);
        println!("  nur StencilCraft: {:?}", extra);
        let mut d: Vec<_> = diff_pkg.into_iter().collect();
        d.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
        for (p, (c, oa, ta)) in d.iter().take(25) {
            println!("  Flächenabweichung {p}: {c} Öffnungen, Ø ours {:.4} / pcbi {:.4} mm²", oa / *c as f64, ta / *c as f64);
        }
    }
}
