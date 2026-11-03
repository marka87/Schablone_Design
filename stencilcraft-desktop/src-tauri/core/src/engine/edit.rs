//! Direkte Bearbeitung von Öffnungen (Board-Auswahl): verschieben, drehen, skalieren, Größe setzen,
//! löschen, hinzufügen, duplizieren. Alles landet als `custom_slices` (Handarbeit) auf den
//! Arbeits-Aperturen; „Zurücksetzen“ und Undo stellen den vorherigen Stand wieder her.

use super::geometry::{bounds, polygon_area, rect_pts, rotate_pts};
use super::types::{r4, round_pts, ApertureItem, PadBox, Pt, Slice};
use super::StencilEngine;

/// Aktuelle Öffnung einer Apertur als Teilstücke (unverändert: die Kundenform).
pub fn effective_slices(ap: &ApertureItem) -> Vec<Slice> {
    if let Some(s) = &ap.custom_slices {
        return s.clone();
    }
    match ap.shape_type.as_str() {
        "circle" => vec![Slice { shape: Some("circle".into()), ..Slice::rect(ap.cx, ap.cy, ap.width, ap.width) }],
        "polygon" if ap.points.len() >= 3 => vec![poly_slice(ap.points.clone(), None)],
        _ => vec![Slice::rect(ap.cx, ap.cy, ap.width, ap.height)],
    }
}

fn poly_slice(pts: Vec<Pt>, template: Option<&Slice>) -> Slice {
    let (x0, y0, x1, y1) = bounds(&pts);
    let mut s = template.cloned().unwrap_or_default();
    s.cx = r4((x0 + x1) / 2.0);
    s.cy = r4((y0 + y1) / 2.0);
    s.w = r4(x1 - x0);
    s.h = r4(y1 - y0);
    s.area = s.area.map(|_| polygon_area(&pts));
    s.shape = Some("polygon".into());
    s.pts = Some(round_pts(&pts));
    s
}

/// Punktabbildung auf ein Teilstück anwenden (`lin` = lineare Maßstäbe x/y für Breite, Höhe, Radius).
fn map_slice(s: &Slice, f: &dyn Fn(Pt) -> Pt, lin: (f64, f64)) -> Slice {
    if let Some(p) = s.polygon_pts() {
        let mut out = poly_slice(p.iter().map(|&q| f(q)).collect(), Some(s));
        out.radius = s.radius.map(|r| r4(r * lin.0.min(lin.1)));
        return out;
    }
    let (cx, cy) = f((s.cx, s.cy));
    let mut out = s.clone();
    out.cx = r4(cx);
    out.cy = r4(cy);
    if s.is_circle() {
        out.w = r4(s.w * (lin.0 * lin.1).sqrt());
        out.h = out.w;
    } else {
        out.w = r4(s.w * lin.0);
        out.h = r4(s.h * lin.1);
    }
    out.d_inner = s.d_inner.map(|d| r4(d * lin.0.min(lin.1)));
    out.d_outer = s.d_outer.map(|d| r4(d * lin.0.min(lin.1)));
    out.area = s.area.map(|a| a * lin.0 * lin.1);
    out
}

/// Teilstück um (ox, oy) drehen; Rechtecke bleiben bei Vielfachen von 90° Rechtecke.
fn rotate_slice(s: &Slice, ox: f64, oy: f64, deg: f64) -> Slice {
    let quarter = (deg / 90.0).round();
    let f = |p: Pt| rotate_pts(&[p], ox, oy, deg)[0];
    if s.polygon_pts().is_some() || s.is_circle() {
        return map_slice(s, &f, (1.0, 1.0));
    }
    if (deg - quarter * 90.0).abs() > 1e-6 {
        return poly_slice(rect_pts(s.cx, s.cy, s.w, s.h).into_iter().map(f).collect(), Some(s));
    }
    let mut out = map_slice(s, &f, (1.0, 1.0));
    if (quarter as i64).rem_euclid(2) == 1 {
        std::mem::swap(&mut out.w, &mut out.h);
    }
    out
}

/// Umschließendes Rechteck aller Teilstücke.
fn slices_bounds(slices: &[Slice]) -> Option<(f64, f64, f64, f64)> {
    let pts: Vec<Pt> = slices
        .iter()
        .flat_map(|s| match s.polygon_pts() {
            Some(p) => p.clone(),
            None => vec![(s.cx - s.w / 2.0, s.cy - s.h / 2.0), (s.cx + s.w / 2.0, s.cy + s.h / 2.0)],
        })
        .collect();
    (!pts.is_empty()).then(|| bounds(&pts))
}

impl StencilEngine {
    fn valid_ids(&self, ids: &[i64]) -> Vec<usize> {
        let mut v: Vec<usize> = ids.iter().filter(|&&i| i >= 0 && (i as usize) < self.apertures.len()).map(|&i| i as usize).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Öffnungen der gewählten Aperturen umbauen; gelöschte (leere) Öffnungen bleiben leer.
    fn edit_openings(&mut self, ids: &[i64], f: impl Fn(&ApertureItem, Vec<Slice>) -> Vec<Slice>) -> usize {
        let ids = self.valid_ids(ids);
        for &id in &ids {
            let ap = &self.apertures[id];
            let slices = f(ap, effective_slices(ap));
            let ap = &mut self.apertures[id];
            ap.custom_slices = Some(slices);
            ap.is_manually_overridden = true;
        }
        ids.len()
    }

    pub fn move_openings(&mut self, ids: &[i64], dx: f64, dy: f64) -> usize {
        self.edit_openings(ids, |_, s| s.iter().map(|x| map_slice(x, &|(px, py)| (px + dx, py + dy), (1.0, 1.0))).collect())
    }

    /// Jede Öffnung um die Mitte ihres Pads drehen (Grad, gegen den Uhrzeigersinn).
    pub fn rotate_openings(&mut self, ids: &[i64], deg: f64) -> usize {
        self.edit_openings(ids, |ap, s| s.iter().map(|x| rotate_slice(x, ap.cx, ap.cy, deg)).collect())
    }

    /// Um die Padmitte skalieren (linear je Achse); Form, Stege und Rundungen bleiben erhalten.
    pub fn scale_openings(&mut self, ids: &[i64], sx: f64, sy: f64) -> usize {
        let (sx, sy) = (sx.max(0.01), sy.max(0.01));
        self.edit_openings(ids, |ap, s| {
            let (ox, oy) = (ap.cx, ap.cy);
            s.iter().map(|x| map_slice(x, &|(px, py)| (ox + (px - ox) * sx, oy + (py - oy) * sy), (sx, sy))).collect()
        })
    }

    /// Fläche um `percent` verkleinern, Form bleibt (statt Rechteck wie früher).
    pub fn reduce_openings(&mut self, ids: &[i64], percent: f64) -> usize {
        let k = (1.0 - percent / 100.0).max(0.01).sqrt();
        self.scale_openings(ids, k, k)
    }

    /// Umschließende Größe der Öffnung auf `w` × `h` mm setzen (None = Achse unverändert).
    pub fn resize_openings(&mut self, ids: &[i64], w: Option<f64>, h: Option<f64>) -> usize {
        let ids = self.valid_ids(ids);
        for id in ids.iter().map(|&i| i as i64) {
            let Some((x0, y0, x1, y1)) = slices_bounds(&effective_slices(&self.apertures[id as usize])) else { continue };
            let sx = w.filter(|v| *v > 0.0 && x1 > x0).map_or(1.0, |v| v / (x1 - x0));
            let sy = h.filter(|v| *v > 0.0 && y1 > y0).map_or(1.0, |v| v / (y1 - y0));
            // um die Mitte der Öffnung, nicht des Pads (verschobene Öffnungen bleiben, wo sie sind)
            let (ox, oy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            self.edit_openings(&[id], |_, s| s.iter().map(|x| map_slice(x, &|(px, py)| (ox + (px - ox) * sx, oy + (py - oy) * sy), (sx, sy))).collect());
        }
        ids.len()
    }

    /// Keine Öffnung mehr an diesen Pads (Zurücksetzen holt sie wieder).
    pub fn delete_openings(&mut self, ids: &[i64]) -> usize {
        self.edit_openings(ids, |_, _| Vec::new())
    }

    /// Neue Öffnung von Hand. Liefert ihre Id.
    // shortcut: Handöffnungen nutzen den Mechanismus der aus dem Kupfer ergänzten Öffnungen (from_copper),
    // damit sie gespeichert werden; „Ergänzte entfernen“ löscht sie mit. Eigenes Flag, falls das stört.
    pub fn add_opening(&mut self, slices: Vec<Slice>) -> Option<usize> {
        let (x0, y0, x1, y1) = slices_bounds(&slices)?;
        let id = self.apertures.len();
        let mut cu = PadBox::new(x0, y0, x1, y1);
        cu.round = slices.len() == 1 && slices[0].is_circle();
        let mut ap = Self::copper_aperture(id, &cu, "", "", 1, "right");
        ap.custom_slices = Some(slices);
        ap.is_manually_overridden = true;
        // Nach Undo kann die Originalliste länger sein: Eintrag dieser Id neu setzen
        self.original_apertures.truncate(id);
        self.original_apertures.push(ap.clone());
        self.apertures.push(ap);
        Some(id)
    }

    pub fn add_opening_shape(&mut self, shape: &str, cx: f64, cy: f64, w: f64, h: f64) -> Option<usize> {
        if !(w > 0.0 && h > 0.0) {
            return None;
        }
        let s = match shape {
            "circle" => Slice { shape: Some("circle".into()), ..Slice::rect(r4(cx), r4(cy), r4(w), r4(w)) },
            _ => Slice::rect(r4(cx), r4(cy), r4(w), r4(h)),
        };
        self.add_opening(vec![s])
    }

    /// Öffnungen kopieren und um (dx, dy) versetzt einfügen. Liefert die neuen Ids.
    pub fn duplicate_openings(&mut self, ids: &[i64], dx: f64, dy: f64) -> Vec<usize> {
        let sources: Vec<Vec<Slice>> = self.valid_ids(ids).into_iter().map(|i| effective_slices(&self.apertures[i])).filter(|s| !s.is_empty()).collect();
        sources
            .into_iter()
            .filter_map(|s| self.add_opening(s.iter().map(|x| map_slice(x, &|(px, py)| (px + dx, py + dy), (1.0, 1.0))).collect()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn engine() -> StencilEngine {
        let mut e = StencilEngine::new(0.12, PathBuf::from("."));
        for (i, (x, w, h)) in [(0.0, 1.0, 0.5), (5.0, 1.0, 0.5)].into_iter().enumerate() {
            e.apertures.push(ApertureItem::new(i, "rectangle", x, 0.0, w, h));
        }
        e.original_apertures = e.apertures.clone();
        e
    }

    fn area(s: &[Slice]) -> f64 {
        s.iter().map(|x| x.pts.as_ref().map_or(x.w * x.h, |p| polygon_area(p))).sum()
    }

    #[test]
    fn reduce_keeps_house_shape() {
        let mut e = engine();
        let house = vec![(-0.5, -0.25), (0.5, -0.25), (0.5, 0.25), (0.0, 0.1), (-0.5, 0.25)];
        e.apertures[0].custom_slices = Some(vec![poly_slice(house.clone(), None)]);
        let before = area(e.apertures[0].custom_slices.as_ref().unwrap());
        assert_eq!(e.reduce_openings(&[0], 19.0), 1);
        let s = e.apertures[0].custom_slices.as_ref().unwrap();
        assert_eq!(s[0].pts.as_ref().unwrap().len(), house.len(), "Form bleibt ein Haus");
        assert!((area(s) / before - 0.81).abs() < 0.01);
        assert!(e.apertures[0].is_manually_overridden);
    }

    #[test]
    fn move_rotate_resize_delete() {
        let mut e = engine();
        e.move_openings(&[0, 1, 99, -1], 0.1, -0.2);
        assert!((e.apertures[1].custom_slices.as_ref().unwrap()[0].cx - 5.1).abs() < 1e-9);
        e.rotate_openings(&[0], 90.0);
        let s = &e.apertures[0].custom_slices.as_ref().unwrap()[0];
        assert!((s.w - 0.5).abs() < 1e-9 && (s.h - 1.0).abs() < 1e-9 && s.pts.is_none());
        assert!((s.cx - 0.2).abs() < 1e-9 && (s.cy - 0.1).abs() < 1e-9, "dreht um die Padmitte");
        e.rotate_openings(&[1], 45.0);
        assert_eq!(e.apertures[1].custom_slices.as_ref().unwrap()[0].pts.as_ref().unwrap().len(), 4);
        e.resize_openings(&[0], Some(0.4), None);
        let s = &e.apertures[0].custom_slices.as_ref().unwrap()[0];
        assert!((s.w - 0.4).abs() < 1e-9 && (s.h - 1.0).abs() < 1e-9 && (s.cx - 0.2).abs() < 1e-9);
        e.delete_openings(&[0]);
        assert!(e.apertures[0].custom_slices.as_ref().unwrap().is_empty());
        e.reset_aperture_modification(0, false);
        assert!(e.apertures[0].custom_slices.is_none());
    }

    #[test]
    fn add_and_duplicate() {
        let mut e = engine();
        let id = e.add_opening_shape("circle", 10.0, 10.0, 0.3, 0.3).unwrap();
        assert_eq!(id, 2);
        assert_eq!(e.duplicate_openings(&[0, 1], 0.0, 2.0), vec![3, 4]);
        assert!((e.apertures[4].custom_slices.as_ref().unwrap()[0].cy - 2.0).abs() < 1e-9);
        assert_eq!(e.add_opening_shape("rect", 0.0, 0.0, 0.0, 1.0), None);
        // Zurücksetzen einer Handöffnung entfernt sie (keine Kundenpaste dort)
        e.reset_aperture_modification(2, false);
        assert!(e.apertures[2].custom_slices.as_ref().unwrap().is_empty());
    }
}
