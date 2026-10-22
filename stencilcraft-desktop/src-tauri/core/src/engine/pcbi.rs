//! Austausch mit dem PCB Investigator Stencil Generator:
//! Kundenbibliothek im Format `RuleTreeManager` (ruleTreeFile_*.xml) lesen und schreiben.
//!
//! Aufbau der Datei (XmlSerializer, .NET):
//! - `AllRuleTrees/RuleTreeEntry`: `PinPackageRules/OnePin`, `ComponentRules`, `PackageRules/OnePackage`, …
//! - `PackageGroupParents/OnePackageGroup`: Regel je Package-Gruppe
//! - `Settings`: Analyse-/Editor-Einstellungen (Längen ohne "MM"-Suffix in mil)
//!
//! Jede Regel besteht aus `UsedRule` (Skriptname) und `RuleSettings` – ein eingebettetes,
//! escaptes XML-Dokument mit den Parametern des Skripts.

use super::types::*;
use super::StencilEngine;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Minimaler XML-Baum (ausreichend für XmlSerializer-Dateien: keine DTD, keine Namespaces nötig)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct XmlNode {
    pub name: String,
    pub text: String,
    pub children: Vec<XmlNode>,
}

impl XmlNode {
    pub fn child(&self, name: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.name == name)
    }
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlNode> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim())
    }
    pub fn f64_of(&self, name: &str) -> Option<f64> {
        self.text_of(name).and_then(|s| s.replace(',', ".").parse().ok())
    }
}

pub fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Parst ein XML-Dokument und liefert das Wurzelelement.
pub fn parse_xml(src: &str) -> Result<XmlNode, String> {
    let src = src.trim_start_matches('\u{feff}');
    let bytes = src.as_bytes();
    let mut stack: Vec<XmlNode> = vec![XmlNode::default()];
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            let end = src[i..].find('<').map_or(src.len(), |e| i + e);
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&xml_unescape(&src[i..end]));
            }
            i = end;
            continue;
        }
        let rest = &src[i..];
        if rest.starts_with("<?") {
            i += rest.find("?>").ok_or("unvollständige XML-Deklaration")? + 2;
        } else if rest.starts_with("<!--") {
            i += rest.find("-->").ok_or("unvollständiger Kommentar")? + 3;
        } else if rest.starts_with("<![CDATA[") {
            let e = rest.find("]]>").ok_or("unvollständiger CDATA-Block")?;
            if let Some(top) = stack.last_mut() {
                top.text.push_str(&rest[9..e]);
            }
            i += e + 3;
        } else if rest.starts_with("<!") {
            i += rest.find('>').ok_or("unvollständige Deklaration")? + 1;
        } else if rest.starts_with("</") {
            let e = rest.find('>').ok_or("unvollständiges End-Tag")?;
            let name = rest[2..e].trim();
            let node = stack.pop().filter(|_| !stack.is_empty()).ok_or("unerwartetes End-Tag")?;
            if node.name != name {
                return Err(format!("End-Tag </{name}> passt nicht zu <{}>", node.name));
            }
            stack.last_mut().ok_or("Strukturfehler")?.children.push(node);
            i += e + 1;
        } else {
            // Start-Tag; Attributwerte können '>' enthalten
            let mut e = 1;
            let mut quote: Option<u8> = None;
            while e < rest.len() {
                let c = rest.as_bytes()[e];
                match quote {
                    Some(q) if c == q => quote = None,
                    Some(_) => {}
                    None if c == b'"' || c == b'\'' => quote = Some(c),
                    None if c == b'>' => break,
                    None => {}
                }
                e += 1;
            }
            if e >= rest.len() {
                return Err("unvollständiges Start-Tag".into());
            }
            let inner = &rest[1..e];
            let self_closing = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name = inner.split(|c: char| c.is_whitespace()).next().unwrap_or("").to_string();
            let node = XmlNode { name, ..Default::default() };
            if self_closing {
                stack.last_mut().ok_or("Strukturfehler")?.children.push(node);
            } else {
                stack.push(node);
            }
            i += e + 1;
        }
    }
    if stack.len() != 1 {
        return Err(format!("Element <{}> nicht geschlossen", stack.last().map_or("", |n| n.name.as_str())));
    }
    stack.pop().and_then(|d| d.children.into_iter().next()).ok_or_else(|| "leeres XML-Dokument".into())
}

// ---------------------------------------------------------------------------
// Regel-Mapping PCB Investigator ⇄ StencilCraft
// ---------------------------------------------------------------------------

pub const RULE_SIMPLE: &str = "Simple Rectangle Surface";
pub const RULE_GROUP: &str = "Handle Stencil Creation by Package Group";
pub const RULE_HOUSE: &str = "HouseBuilder";
pub const RULE_TAKE: &str = "Take Existing Paste Elements";
pub const RULE_IGNORE: &str = "Ignore";

/// PCB-I-Gruppenname → StencilCraft-Gruppenschlüssel
pub fn group_key(pcbi: &str) -> Option<&'static str> {
    match pcbi.trim() {
        "SMD Packages" => Some("SMD"),
        "THT Packages" => Some("THT"),
        "Other Packages" => Some("Other"),
        "Fiducials" | "Fiducial Packages" => Some("Fiducial"),
        _ => None,
    }
}

pub fn group_name(key: &str) -> Option<&'static str> {
    match key {
        "SMD" => Some("SMD Packages"),
        "THT" => Some("THT Packages"),
        "Other" => Some("Other Packages"),
        "Fiducial" => Some("Fiducials"),
        _ => None,
    }
}

/// Wandelt `UsedRule` + Parameter-XML in eine PackageRule.
/// `None` als Regeltyp-Hinweis: unbekanntes Skript (wird als "Original übernehmen" geführt).
/// `inherit_if_empty`: leere Parameter bei "Package Group" bedeuten Gruppenvorgabe.
pub fn rule_from_pcbi(used: &str, settings_xml: &str, inherit_if_empty: bool) -> (PackageRule, bool) {
    let settings = if settings_xml.trim().is_empty() { None } else { parse_xml(settings_xml).ok() };
    let s = settings.as_ref();
    let f = |k: &str| s.and_then(|n| n.f64_of(k));
    let b = |k: &str| s.and_then(|n| n.text_of(k)).map(|v| v.eq_ignore_ascii_case("true"));
    let mut r: PackageRule;
    let mut known = true;
    match used.trim() {
        RULE_SIMPLE => {
            r = PackageRule::with("simple_rect", RULE_SIMPLE);
            // PCB-I kennt hier kein Undersize; "OffsetMM" vergrößert die Öffnung allseitig
            let off = f("OffsetMM").unwrap_or(0.0);
            r.undersize_w = -off;
            r.undersize_h = -off;
            r.corner_radius = f("CornerRadiusMM").unwrap_or(r.corner_radius);
            r.try_find_direction = b("TryFindDirection").unwrap_or(true);
            r.rotate_bridges_45 = b("RotateSurfaceBridges45Degree").unwrap_or(false);
            r.offset_x = f("OffsetToCenterXMM").unwrap_or(0.0);
            r.offset_y = f("OffsetToCenterYMM").unwrap_or(0.0);
            r.bridge_width = f("BridgeWidthMM").unwrap_or(r.bridge_width);
            r.bridge_count_x = f("BridgeCountX").unwrap_or(0.0).round().max(0.0) as i64;
            r.bridge_count_y = f("BridgeCountY").unwrap_or(0.0).round().max(0.0) as i64;
            r.size_w = f("SizeWidthMM").unwrap_or(0.0).max(0.0);
            r.size_h = f("SizeHeightMM").unwrap_or(0.0).max(0.0);
        }
        RULE_GROUP => {
            if s.is_none() && inherit_if_empty {
                return (PackageRule::with("inherit", "Inherit"), true);
            }
            r = PackageRule::with("package_group", RULE_GROUP);
            r.house_cut_pct = f("CutSizeValuePercent").unwrap_or(r.house_cut_pct);
            r.corner_radius = f("CornerRadiusMM").unwrap_or(r.corner_radius);
            r.mini_chip_reduction = f("MiniChipSizeMM").unwrap_or(r.mini_chip_reduction);
            r.bga_threshold_square = f("BGAThreasholdSquareMM").unwrap_or(r.bga_threshold_square);
            let us = f("UndersizeValueMM").unwrap_or(r.undersize_w);
            r.undersize_w = us;
            r.undersize_h = us;
            r.chip_threshold_houses = f("ChipThresholdHousesMM").unwrap_or(r.chip_threshold_houses);
            r.min_array_field = f("MinArrayFieldSize").unwrap_or(r.min_array_field);
            r.distance_array_fields = f("DistanceArrayFieldSize").unwrap_or(r.distance_array_fields);
            r.utilization_array_pct = f("UtilizationPercent").unwrap_or(r.utilization_array_pct);
            r.utilization_house_pct = f("UtilizationHousePercent").unwrap_or(r.utilization_house_pct);
            r.offset_move_away = f("OffsetToCenterMM").unwrap_or(0.0);
            r.invert_house_below = f("InvertHouseSmallerThan").unwrap_or(r.invert_house_below);
        }
        RULE_HOUSE => {
            let invert = b("InvertDirection").unwrap_or(false);
            r = if invert { PackageRule::with("inverted_house_builder", RULE_HOUSE) } else { PackageRule::with("house_builder", RULE_HOUSE) };
            // Vorgaben des PCB-I-Skripts, wenn keine Parameter gespeichert sind (aus Referenzjob ermittelt)
            r.undersize_w = 0.01;
            r.undersize_h = 0.01;
            r.house_cut_pct = 30.0;
            r.corner_radius = f("CornerRadiusMM").unwrap_or(r.corner_radius);
            r.offset_move_away = f("OffsetToCenterMM").unwrap_or(0.0);
            let us = f("UndersizeValueMM").unwrap_or(r.undersize_w);
            r.undersize_w = us;
            r.undersize_h = us;
            r.house_cut_pct = f("CutSizeValuePercent").unwrap_or(r.house_cut_pct);
            r.size_w = f("FixedWidthMM").unwrap_or(0.0).max(0.0);
            r.size_h = f("FixedHeightMM").unwrap_or(0.0).max(0.0);
            r.utilization_house_pct = f("UtilizationPercent").unwrap_or(0.0);
            if let Some(v) = f("SideOffsetMM").filter(|v| *v != 0.0) {
                r.extra.insert("SideOffsetMM".into(), fmt_num(v));
            }
        }
        RULE_TAKE => r = PackageRule::with("take_existing", RULE_TAKE),
        RULE_IGNORE => r = PackageRule::with("ignore", RULE_IGNORE),
        other => {
            known = false;
            r = PackageRule::with("take_existing", other);
        }
    }
    (r, known)
}

/// Zahl wie .NET XmlSerializer (ohne unnötige Nachkommastellen).
pub fn fmt_num(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r}")
}

const NS: &str = r#"xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema""#;

fn settings_doc(root: &str, fields: &[(&str, String)]) -> String {
    let mut s = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<{root} {NS}>\r\n");
    for (k, v) in fields {
        s.push_str(&format!("  <{k}>{v}</{k}>\r\n"));
    }
    s.push_str(&format!("</{root}>"));
    s
}

/// PackageRule → (`UsedRule`, Parameter-XML). `None` = keine PCB-I-Entsprechung.
pub fn rule_to_pcbi(r: &PackageRule) -> Option<(&'static str, String)> {
    let n = fmt_num;
    let bl = |b: bool| if b { "true".to_string() } else { "false".to_string() };
    match r.rule_type.as_str() {
        "simple_rect" | "array_slicing" | "array_pads" => Some((
            RULE_SIMPLE,
            settings_doc(
                "StencilScriptSimpleSurface",
                &[
                    ("CornerRadiusMM", n(r.corner_radius)),
                    ("TryFindDirection", bl(r.try_find_direction)),
                    ("RotateSurfaceBridges45Degree", bl(r.rotate_bridges_45)),
                    ("OffsetToCenterXMM", n(r.offset_x)),
                    ("OffsetToCenterYMM", n(r.offset_y)),
                    ("BridgeWidthMM", n(r.bridge_width)),
                    ("BridgeCountX", r.bridge_count_x.to_string()),
                    ("BridgeCountY", r.bridge_count_y.to_string()),
                    ("OffsetMM", n(-r.undersize_w)),
                    ("SizeWidthMM", n(r.size_w)),
                    ("SizeHeightMM", n(r.size_h)),
                ],
            ),
        )),
        "package_group" => Some((
            RULE_GROUP,
            settings_doc(
                "StencilScriptHandleByPackageGroup",
                &[
                    ("CutSizeValuePercent", n(r.house_cut_pct)),
                    ("CornerRadiusMM", n(r.corner_radius)),
                    ("MiniChipSizeMM", n(r.mini_chip_reduction)),
                    ("BGAThreasholdSquareMM", n(r.bga_threshold_square)),
                    ("UndersizeValueMM", n(r.undersize_w)),
                    ("ChipThresholdHousesMM", n(r.chip_threshold_houses)),
                    ("MinArrayFieldSize", n(r.min_array_field)),
                    ("DistanceArrayFieldSize", n(r.distance_array_fields)),
                    ("UtilizationPercent", n(r.utilization_array_pct)),
                    ("UtilizationHousePercent", n(r.utilization_house_pct)),
                    ("OffsetToCenterMM", n(r.offset_move_away)),
                    ("InvertHouseSmallerThan", n(r.invert_house_below)),
                ],
            ),
        )),
        "house_builder" | "inverted_house_builder" => Some((
            RULE_HOUSE,
            settings_doc(
                "StencilScriptHouse",
                &[
                    ("CornerRadiusMM", n(r.corner_radius)),
                    ("OffsetToCenterMM", n(r.offset_move_away)),
                    ("SideOffsetMM", r.extra.get("SideOffsetMM").cloned().unwrap_or_else(|| "0".into())),
                    ("InvertDirection", bl(r.rule_type == "inverted_house_builder")),
                    ("UndersizeValueMM", n(r.undersize_w)),
                    ("CutSizeValuePercent", n(r.house_cut_pct)),
                    ("FixedWidthMM", n(r.size_w)),
                    ("FixedHeightMM", n(r.size_h)),
                    ("UtilizationPercent", n(r.utilization_house_pct)),
                ],
            ),
        )),
        "take_existing" => Some((RULE_TAKE, String::new())),
        "ignore" => Some((RULE_IGNORE, settings_doc("StencilScriptIgnore", &[]).replace(&format!(">\r\n</StencilScriptIgnore>"), " />"))),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Datei-Modell
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
pub struct RuleTree {
    /// Package → (Gruppe, Regel)
    pub packages: BTreeMap<String, (String, PackageRule)>,
    /// Package → Pin (1-basiert) → Regel
    pub pins: BTreeMap<String, BTreeMap<i64, PackageRule>>,
    /// Bauteil (RefDes) → Regel
    pub components: BTreeMap<String, PackageRule>,
    /// Gruppenschlüssel → Regel
    pub groups: BTreeMap<String, PackageRule>,
    /// Unveränderter `<Settings>`-Block (für verlustfreien Export)
    pub settings_xml: Option<String>,
    /// Standard-Blechdicke in mm (EditorSetting/DefaultThickness, in mil gespeichert)
    pub default_thickness: Option<f64>,
    pub area_ratio: Option<f64>,
    pub aspect_ratio: Option<f64>,
    pub unknown_rules: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
}

const MIL: f64 = 0.0254;

/// Schneidet einen Elementblock roh aus dem Quelltext (für verlustfreie Übernahme von `<Settings>`).
fn raw_block(src: &str, name: &str) -> Option<String> {
    let start = src.find(&format!("<{name}>"))?;
    let end_tag = format!("</{name}>");
    let end = src[start..].find(&end_tag)? + start + end_tag.len();
    Some(src[start..end].to_string())
}

pub fn parse_rule_tree(src: &str) -> Result<RuleTree, String> {
    let root = parse_xml(src)?;
    if root.name != "RuleTreeManager" {
        return Err(format!("Keine PCB-Investigator-Kundenbibliothek (Wurzel <{}> statt <RuleTreeManager>)", root.name));
    }
    let mut t = RuleTree::default();
    let note = |t: &mut RuleTree, used: &str, known: bool| {
        if !known {
            *t.unknown_rules.entry(used.to_string()).or_default() += 1;
        }
    };
    let entries: Vec<&XmlNode> = root.child("AllRuleTrees").map(|a| a.children_named("RuleTreeEntry").collect()).unwrap_or_default();
    if entries.len() > 1 {
        t.warnings.push(format!("{} Regelbäume (Steps) in der Datei – alle werden zusammengeführt", entries.len()));
    }
    for e in entries {
        if let Some(pp) = e.child("PackageRules") {
            for p in pp.children_named("OnePackage") {
                let used = p.text_of("UsedRule").unwrap_or("");
                let (rule, known) = rule_from_pcbi(used, p.text_of("RuleSettings").unwrap_or(""), true);
                note(&mut t, used, known);
                let Some(name) = p.text_of("PackageName").filter(|n| !n.is_empty()) else { continue };
                let group = p.text_of("PackageGroup").and_then(group_key).unwrap_or("SMD").to_string();
                t.packages.insert(name.to_string(), (group, rule));
            }
        }
        for (block, item) in [("PinPackageRules", "OnePin"), ("PinComponentRules", "OnePin")] {
            let Some(pp) = e.child(block) else { continue };
            for p in pp.children_named(item) {
                let used = p.text_of("UsedRule").unwrap_or("");
                let (rule, known) = rule_from_pcbi(used, p.text_of("RuleSettings").unwrap_or(""), true);
                note(&mut t, used, known);
                let Some(name) = p.text_of("ComponentOrPackageName").filter(|n| !n.is_empty()) else { continue };
                let Some(idx) = p.f64_of("PinIndex") else { continue };
                if block == "PinComponentRules" {
                    t.warnings.push(format!("Pin-Regel für Bauteil {name} (Pin {}) wird als Package-Pin-Regel nicht unterstützt", idx as i64 + 1));
                    continue;
                }
                // PCB-I zählt Pins ab 0, StencilCraft (ODB++-Toeprints) ab 1
                t.pins.entry(name.to_string()).or_default().insert(idx as i64 + 1, rule);
            }
        }
        if let Some(cc) = e.child("ComponentRules") {
            for c in &cc.children {
                let used = c.text_of("UsedRule").unwrap_or("");
                let (rule, known) = rule_from_pcbi(used, c.text_of("RuleSettings").unwrap_or(""), false);
                note(&mut t, used, known);
                let name = ["ComponentName", "ComponentOrPackageName", "Name"].iter().find_map(|k| c.text_of(k)).filter(|n| !n.is_empty());
                match name {
                    Some(n) => {
                        t.components.insert(n.to_string(), rule);
                    }
                    None => t.warnings.push("Bauteilregel ohne Bauteilnamen übersprungen".into()),
                }
            }
        }
        if e.child("ZoneRules").is_some_and(|z| !z.children.is_empty()) {
            t.warnings.push("Zonenregeln aus PCB-I werden noch nicht übernommen (Zonen bitte im Zonen-Editor anlegen)".into());
        }
    }
    if let Some(gp) = root.child("PackageGroupParents") {
        for g in gp.children_named("OnePackageGroup") {
            let used = g.text_of("UsedRule").unwrap_or("");
            let (mut rule, known) = rule_from_pcbi(used, g.text_of("RuleSettings").unwrap_or(""), false);
            note(&mut t, used, known);
            if let Some(k) = g.text_of("PackageGroupName").and_then(group_key) {
                if rule.rule_type == "inherit" {
                    rule = PackageRule::with("package_group", RULE_GROUP);
                }
                t.groups.insert(k.to_string(), rule);
            }
        }
    }
    if let Some(s) = root.child("Settings") {
        t.settings_xml = raw_block(src, "Settings");
        let a = s.child("AnalysisSettings");
        t.area_ratio = a.and_then(|a| a.f64_of("AreaRatio"));
        t.aspect_ratio = a.and_then(|a| a.f64_of("AspectRatio"));
        t.default_thickness = s.child("EditorSetting").and_then(|e| e.f64_of("DefaultThickness")).map(|mil| (mil * MIL * 1000.0).round() / 1000.0);
    }
    Ok(t)
}

fn one_rule_xml(indent: &str, used: &str, settings: &str) -> String {
    let rs = if settings.is_empty() { format!("{indent}<RuleSettings />\r\n") } else { format!("{indent}<RuleSettings>{}</RuleSettings>\r\n", xml_escape(settings)) };
    format!("{indent}<UsedRule>{}</UsedRule>\r\n{rs}", xml_escape(used))
}

pub fn write_rule_tree(t: &RuleTree) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n");
    x.push_str(&format!("<RuleTreeManager {NS}>\r\n  <AllRuleTrees>\r\n    <RuleTreeEntry>\r\n      <StepName />\r\n"));
    let pin_count: usize = t.pins.values().map(|p| p.len()).sum();
    if pin_count == 0 {
        x.push_str("      <PinPackageRules />\r\n");
    } else {
        x.push_str("      <PinPackageRules>\r\n");
        for (pkg, pins) in &t.pins {
            for (pin, r) in pins {
                let Some((used, set)) = rule_to_pcbi(r) else { continue };
                x.push_str("        <OnePin>\r\n");
                x.push_str(&one_rule_xml("          ", used, &set));
                x.push_str(&format!(
                    "          <ClassType>Pin</ClassType>\r\n          <ComponentOrPackageName>{}</ComponentOrPackageName>\r\n          <PinIndex>{}</PinIndex>\r\n        </OnePin>\r\n",
                    xml_escape(pkg),
                    (pin - 1).max(0)
                ));
            }
        }
        x.push_str("      </PinPackageRules>\r\n");
    }
    x.push_str("      <PinComponentRules />\r\n");
    if t.components.is_empty() {
        x.push_str("      <ComponentRules />\r\n");
    } else {
        x.push_str("      <ComponentRules>\r\n");
        for (c, r) in &t.components {
            let Some((used, set)) = rule_to_pcbi(r) else { continue };
            x.push_str("        <OneComponent>\r\n");
            x.push_str(&one_rule_xml("          ", used, &set));
            x.push_str(&format!("          <ClassType>Component</ClassType>\r\n          <ComponentName>{}</ComponentName>\r\n        </OneComponent>\r\n", xml_escape(c)));
        }
        x.push_str("      </ComponentRules>\r\n");
    }
    x.push_str("      <ZoneRules />\r\n");
    if t.packages.is_empty() {
        x.push_str("      <PackageRules />\r\n");
    } else {
        x.push_str("      <PackageRules>\r\n");
        for (name, (group, r)) in &t.packages {
            let (used, set) = if r.rule_type == "inherit" { (RULE_GROUP, String::new()) } else {
                match rule_to_pcbi(r) {
                    Some((u, s)) => (u, s),
                    None => continue,
                }
            };
            x.push_str("        <OnePackage>\r\n");
            x.push_str(&one_rule_xml("          ", used, &set));
            x.push_str(&format!(
                "          <ClassType>Package</ClassType>\r\n          <PackageName>{}</PackageName>\r\n          <PackageGroup>{}</PackageGroup>\r\n        </OnePackage>\r\n",
                xml_escape(name),
                group_name(group).unwrap_or("SMD Packages")
            ));
        }
        x.push_str("      </PackageRules>\r\n");
    }
    x.push_str("      <ZoneGroupParents />\r\n      <FiducialRules />\r\n      <DummyRules />\r\n      <DummyPins />\r\n      <DummyPackagePins />\r\n    </RuleTreeEntry>\r\n  </AllRuleTrees>\r\n");
    x.push_str("  <PackageGroupParents>\r\n");
    for key in ["SMD", "THT", "Other", "Fiducial"] {
        let Some(r) = t.groups.get(key) else { continue };
        let Some((used, set)) = rule_to_pcbi(r) else { continue };
        x.push_str("    <OnePackageGroup>\r\n");
        x.push_str(&one_rule_xml("      ", used, &set));
        x.push_str(&format!("      <ClassType>PackageGroup</ClassType>\r\n      <PackageGroupName>{}</PackageGroupName>\r\n    </OnePackageGroup>\r\n", group_name(key).unwrap_or(key)));
    }
    x.push_str("  </PackageGroupParents>\r\n");
    match &t.settings_xml {
        Some(s) => x.push_str(&format!("  {s}\r\n")),
        None => {
            x.push_str("  <Settings>\r\n");
            if let Some(th) = t.default_thickness {
                x.push_str(&format!("    <EditorSetting>\r\n      <DefaultThickness>{}</DefaultThickness>\r\n    </EditorSetting>\r\n", fmt_num(th / MIL)));
            }
            x.push_str(&format!("    <DefaultRuleScript>{RULE_GROUP}</DefaultRuleScript>\r\n  </Settings>\r\n"));
        }
    }
    x.push_str("  <PreDefinedPartZones />\r\n</RuleTreeManager>");
    x
}

// ---------------------------------------------------------------------------
// Anbindung an die Engine
// ---------------------------------------------------------------------------

impl StencilEngine {
    /// Wendet eine PCB-I-Kundenbibliothek auf den geladenen Datensatz an.
    /// Liefert einen Bericht: übernommene Packages, Packages ohne Regel ("fehlt in Bibliothek") usw.
    pub fn apply_rule_tree(&mut self, t: &RuleTree, apply_settings: bool) -> Value {
        let mut matched = Vec::new();
        let mut not_in_dataset = Vec::new();
        for (name, (group, rule)) in &t.packages {
            let real = self.resolve_alias(name);
            match self.packages.get_mut(&real) {
                Some(pkg) => {
                    // Die Gruppe aus der Bibliothek ist gepflegtes Wissen und schlägt die automatische Erkennung
                    pkg.group = group.clone();
                    pkg.rule = rule.clone();
                    pkg.pad_overrides.clear();
                    matched.push(real);
                }
                None => not_in_dataset.push(name.clone()),
            }
        }
        let mut pin_rules = 0;
        for (name, pins) in &t.pins {
            let real = self.resolve_alias(name);
            let Some(pkg) = self.packages.get_mut(&real) else {
                if !t.packages.contains_key(name) {
                    not_in_dataset.push(name.clone());
                }
                continue;
            };
            if !t.packages.contains_key(name) {
                pkg.pad_overrides.clear();
            }
            for (pin, r) in pins {
                if let Some(m) = r.to_json().as_object() {
                    pkg.pad_overrides.insert(*pin, m.clone());
                    pin_rules += 1;
                }
            }
        }
        for (g, r) in &t.groups {
            self.group_rules.insert(g.clone(), r.clone());
        }
        let refdes: std::collections::BTreeSet<&str> = self.components.iter().map(|c| c.refdes.as_str()).collect();
        let mut comp_applied = 0;
        for (c, r) in &t.components {
            if refdes.contains(c.as_str()) {
                self.component_rules.insert(c.clone(), r.clone());
                comp_applied += 1;
            }
        }
        if apply_settings {
            if let Some(th) = t.default_thickness.filter(|v| *v > 0.03 && *v < 1.0) {
                self.base_thickness = th;
            }
        }
        self.pcbi_settings_xml = t.settings_xml.clone();
        // Packages im Datensatz ohne Bibliothekseintrag (PCB-I zeigt sie rot)
        let known: std::collections::BTreeSet<String> = t.packages.keys().chain(t.pins.keys()).map(|n| self.resolve_alias(n)).collect();
        let mut missing: Vec<Value> = self
            .packages
            .values()
            .filter(|p| !known.contains(&p.name) && p.group != "Fiducial")
            .map(|p| json!({"name": p.name, "group": p.group, "pins": p.pin_count, "count": p.component_count}))
            .collect();
        missing.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        not_in_dataset.sort();
        not_in_dataset.dedup();
        json!({
            "library_packages": t.packages.len(),
            "matched": matched,
            "not_in_dataset": not_in_dataset,
            "missing_in_library": missing,
            "pin_rules": pin_rules,
            "group_rules": t.groups.keys().collect::<Vec<_>>(),
            "component_rules": comp_applied,
            "unknown_rules": t.unknown_rules,
            "warnings": t.warnings,
            "default_thickness": t.default_thickness,
            "area_ratio": t.area_ratio,
            "aspect_ratio": t.aspect_ratio,
            "base_thickness": self.base_thickness,
        })
    }

    pub fn import_rule_tree_file(&mut self, path: &Path, apply_settings: bool) -> Result<(Value, RuleTree), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let src = String::from_utf8_lossy(&bytes);
        let t = parse_rule_tree(&src)?;
        let mut report = self.apply_rule_tree(&t, apply_settings);
        report["file"] = json!(path.to_string_lossy());
        Ok((report, t))
    }

    /// Speichert die Regeln als StencilCraft-Kundenbibliothek – inklusive aller Packages der
    /// PCB-I-Datei, die im aktuellen Datensatz nicht vorkommen.
    pub fn save_rule_tree_to_library(&mut self, customer: &str, t: &RuleTree) -> Result<String, String> {
        let file = self.save_customer_library(customer)?;
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let mut lib: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        for (name, (_, rule)) in &t.packages {
            if !self.packages.contains_key(&self.resolve_alias(name)) {
                lib["packages"][name] = rule.to_json();
            }
        }
        for (name, pins) in &t.pins {
            if !self.packages.contains_key(&self.resolve_alias(name)) {
                let m: serde_json::Map<String, Value> = pins.iter().map(|(p, r)| (p.to_string(), r.to_json())).collect();
                lib["pin_overrides"][name] = Value::Object(m);
            }
        }
        let out = serde_json::to_string_pretty(&lib).map_err(|e| e.to_string())?;
        super::project::write_atomic(Path::new(&file), &out)?;
        Ok(file)
    }

    /// Aktuelle Regeln als PCB-I-Regelbaum (Packages mit eigener Regel, Pin-Regeln, Gruppen, Bauteile).
    pub fn current_rule_tree(&self) -> (RuleTree, Vec<String>) {
        let mut t = RuleTree { settings_xml: self.pcbi_settings_xml.clone(), default_thickness: Some(self.base_thickness), ..Default::default() };
        let mut skipped = Vec::new();
        let mut check = |what: String, r: &PackageRule| {
            if r.rule_type != "inherit" && rule_to_pcbi(r).is_none() {
                skipped.push(format!("{what}: Regel \"{}\" hat keine PCB-I-Entsprechung", r.rule_type));
            }
        };
        for (name, pkg) in &self.packages {
            if pkg.group == "Library" && pkg.rule.rule_type == "inherit" {
                continue;
            }
            check(format!("Package {name}"), &pkg.rule);
            if pkg.rule.rule_type != "inherit" {
                t.packages.insert(name.clone(), (if pkg.group == "Library" { "SMD".into() } else { pkg.group.clone() }, pkg.rule.clone()));
            }
            let (base, _) = self.package_effective_rule(pkg);
            for (pin, ov) in &pkg.pad_overrides {
                let r = super::rules::resolve_pin_rule(&base, ov);
                check(format!("Package {name} Pin {pin}"), &r);
                t.pins.entry(name.clone()).or_default().insert(*pin, r);
            }
        }
        for (c, r) in &self.component_rules {
            check(format!("Bauteil {c}"), r);
            t.components.insert(c.clone(), r.clone());
        }
        for (g, r) in &self.group_rules {
            if group_name(g).is_some() {
                check(format!("Gruppe {g}"), r);
                t.groups.insert(g.clone(), r.clone());
            }
        }
        (t, skipped)
    }

    pub fn export_rule_tree_file(&self, path: &Path) -> Result<Value, String> {
        let (t, skipped) = self.current_rule_tree();
        let xml = write_rule_tree(&t);
        std::fs::write(path, xml.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(json!({
            "status": "ok",
            "path": path.to_string_lossy(),
            "packages": t.packages.len(),
            "pin_rules": t.pins.values().map(|p| p.len()).sum::<usize>(),
            "groups": t.groups.len(),
            "components": t.components.len(),
            "skipped": skipped,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<RuleTreeManager xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema">
  <AllRuleTrees>
    <RuleTreeEntry>
      <StepName />
      <PinPackageRules>
        <OnePin>
          <UsedRule>Simple Rectangle Surface</UsedRule>
          <RuleSettings>&lt;?xml version="1.0" encoding="utf-8"?&gt;
&lt;StencilScriptSimpleSurface&gt;
  &lt;CornerRadiusMM&gt;0.075&lt;/CornerRadiusMM&gt;
  &lt;BridgeCountX&gt;1&lt;/BridgeCountX&gt;
  &lt;SizeWidthMM&gt;0.2&lt;/SizeWidthMM&gt;
  &lt;SizeHeightMM&gt;1&lt;/SizeHeightMM&gt;
&lt;/StencilScriptSimpleSurface&gt;</RuleSettings>
          <ClassType>Pin</ClassType>
          <ComponentOrPackageName>CON_A</ComponentOrPackageName>
          <PinIndex>0</PinIndex>
        </OnePin>
      </PinPackageRules>
      <PackageRules>
        <OnePackage>
          <UsedRule>HouseBuilder</UsedRule>
          <RuleSettings>&lt;StencilScriptHouse&gt;&lt;UndersizeValueMM&gt;0.005&lt;/UndersizeValueMM&gt;&lt;CutSizeValuePercent&gt;30&lt;/CutSizeValuePercent&gt;&lt;InvertDirection&gt;true&lt;/InvertDirection&gt;&lt;/StencilScriptHouse&gt;</RuleSettings>
          <ClassType>Package</ClassType>
          <PackageName>K0201_HD</PackageName>
          <PackageGroup>SMD Packages</PackageGroup>
        </OnePackage>
        <OnePackage>
          <UsedRule>Handle Stencil Creation by Package Group</UsedRule>
          <RuleSettings />
          <ClassType>Package</ClassType>
          <PackageName>QFN</PackageName>
          <PackageGroup>Other Packages</PackageGroup>
        </OnePackage>
        <OnePackage>
          <UsedRule>Fancy Script</UsedRule>
          <RuleSettings />
          <PackageName>X</PackageName>
        </OnePackage>
      </PackageRules>
    </RuleTreeEntry>
  </AllRuleTrees>
  <PackageGroupParents>
    <OnePackageGroup>
      <UsedRule>Ignore</UsedRule>
      <RuleSettings>&lt;StencilScriptIgnore /&gt;</RuleSettings>
      <PackageGroupName>THT Packages</PackageGroupName>
    </OnePackageGroup>
  </PackageGroupParents>
  <Settings>
    <AnalysisSettings><AreaRatio>0.7</AreaRatio></AnalysisSettings>
    <EditorSetting><DefaultThickness>5.118110236220473</DefaultThickness></EditorSetting>
  </Settings>
</RuleTreeManager>"#;

    #[test]
    fn parse_sample() {
        let t = parse_rule_tree(SAMPLE).unwrap();
        let (g, k) = &t.packages["K0201_HD"];
        assert_eq!(g, "SMD");
        assert_eq!(k.rule_type, "inverted_house_builder");
        assert!((k.undersize_w - 0.005).abs() < 1e-9 && (k.house_cut_pct - 30.0).abs() < 1e-9);
        assert_eq!(t.packages["QFN"].0, "Other");
        assert_eq!(t.packages["QFN"].1.rule_type, "inherit");
        assert_eq!(t.unknown_rules.get("Fancy Script"), Some(&1));
        let pin = &t.pins["CON_A"][&1];
        assert_eq!(pin.rule_type, "simple_rect");
        assert_eq!((pin.size_w, pin.size_h, pin.bridge_count_x), (0.2, 1.0, 1));
        assert_eq!(t.groups["THT"].rule_type, "ignore");
        assert_eq!(t.default_thickness, Some(0.13));
        assert_eq!(t.area_ratio, Some(0.7));
    }

    #[test]
    fn roundtrip() {
        let t = parse_rule_tree(SAMPLE).unwrap();
        let x = write_rule_tree(&t);
        let t2 = parse_rule_tree(&x).unwrap();
        assert_eq!(t2.packages.len(), t.packages.len());
        assert_eq!(t2.packages["X"].1.rule_type, "take_existing");
        assert_eq!(t2.packages["K0201_HD"].1.rule_type, "inverted_house_builder");
        assert_eq!(t2.packages["QFN"].1.rule_type, "inherit");
        assert_eq!(t2.pins["CON_A"][&1].size_w, 0.2);
        assert_eq!(t2.groups["THT"].rule_type, "ignore");
        assert_eq!(t2.area_ratio, Some(0.7));
    }

    #[test]
    fn xml_basics() {
        let n = parse_xml("<a x=\"1>2\"><b>&lt;c&gt; &amp; &#65;</b><d/><![CDATA[<raw>]]></a>").unwrap();
        assert_eq!(n.text_of("b"), Some("<c> & A"));
        assert!(n.child("d").is_some());
        assert_eq!(n.text, "<raw>");
        assert!(parse_xml("<a><b></a>").is_err());
    }
}
