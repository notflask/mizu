//! Strokes <-> PDF ink annotations.
//!
//! Every mizu stroke is a regular `/Ink` annotation named `mizu-<uuid>`
//! (`/NM`), so other viewers show it too and mizu can find its own work
//! again. Pen strokes additionally carry `/MizuW` (base width) and `/MizuP`
//! (pressure per point) plus a hand-written appearance stream so that the
//! pressure shape survives in other viewers.

use std::io::Write as _;
use std::path::Path;

use mupdf::color::AnnotationColor;
use mupdf::pdf::{PdfAnnotation, PdfAnnotationType, PdfDocument, PdfObject, PdfPage};
use mupdf::{Buffer, Point};
use uuid::Uuid;

use crate::ink::stroke::{pressure_width, Stroke};

pub const NM_PREFIX: &str = "mizu-";

pub fn is_mizu(a: &PdfAnnotation) -> bool {
    if !matches!(a.r#type(), Ok(PdfAnnotationType::Ink)) {
        return false;
    }
    name_of(a)
        .map(|n| n.starts_with(NM_PREFIX))
        .unwrap_or(false)
}

fn name_of(a: &PdfAnnotation) -> Option<String> {
    a.object().get_dict("NM").ok().flatten()?.as_string().ok()
}

/// Read all mizu strokes from one page.
pub fn read_strokes(page_index: usize, page: &PdfPage) -> Vec<Stroke> {
    let mut out = Vec::new();
    for a in page.annotations() {
        if !is_mizu(&a) {
            continue;
        }
        let Some(name) = name_of(&a) else { continue };
        let id =
            Uuid::parse_str(name.trim_start_matches(NM_PREFIX)).unwrap_or_else(|_| Uuid::new_v4());
        let Ok(lists) = a.ink_list() else { continue };
        let Some(first) = lists.into_iter().next() else {
            continue;
        };
        if first.is_empty() {
            continue;
        }
        let points: Vec<[f32; 2]> = first.iter().map(|p| [p.x, p.y]).collect();
        let obj = a.object();
        let base_width = obj
            .get_dict("MizuW")
            .ok()
            .flatten()
            .and_then(|o| o.as_float().ok())
            .or_else(|| a.border_width().ok())
            .filter(|w| w.is_finite() && *w > 0.0)
            .unwrap_or(1.5);
        let pressure = obj.get_dict("MizuP").ok().flatten().and_then(|arr| {
            let it = arr.array_iter().ok()?;
            let v: Vec<f32> = it.filter_map(|o| o.ok()?.as_float().ok()).collect();
            (v.len() == points.len()).then_some(v)
        });
        let color = match a.color() {
            Ok(Some(AnnotationColor::Rgb { red, green, blue })) => [
                (red * 255.0).round().clamp(0.0, 255.0) as u8,
                (green * 255.0).round().clamp(0.0, 255.0) as u8,
                (blue * 255.0).round().clamp(0.0, 255.0) as u8,
            ],
            Ok(Some(AnnotationColor::Gray(g))) => {
                let v = (g * 255.0).round().clamp(0.0, 255.0) as u8;
                [v, v, v]
            }
            _ => [0, 0, 0],
        };
        let mut s = Stroke::new(page_index, points, pressure, base_width, color);
        s.id = id;
        out.push(s);
    }
    out
}

/// Remove all mizu annotations from `page`. Returns how many were removed.
pub fn remove_mizu(page: &mut PdfPage) -> Result<usize, mupdf::Error> {
    let doomed: Vec<PdfAnnotation> = page.annotations().filter(is_mizu).collect();
    let n = doomed.len();
    for a in doomed {
        page.delete_annotation(a)?;
    }
    Ok(n)
}

/// Outline of a (possibly pressure sensitive) stroke as convex polygons that
/// all share one orientation, so filling them with the non-zero rule gives
/// their union.
pub fn stroke_polygons(s: &Stroke) -> Vec<Vec<[f32; 2]>> {
    const SIDES: usize = 14;
    let mut polys: Vec<Vec<[f32; 2]>> = Vec::new();
    let radius = |i: usize| -> f32 {
        let w = match &s.pressure {
            Some(p) => pressure_width(s.width, p.get(i).copied().unwrap_or(1.0)),
            None => s.width,
        };
        (w * 0.5).max(0.01)
    };
    for (i, p) in s.points.iter().enumerate() {
        let r = radius(i);
        let poly: Vec<[f32; 2]> = (0..SIDES)
            .map(|k| {
                let a = k as f32 / SIDES as f32 * std::f32::consts::TAU;
                [p[0] + r * a.cos(), p[1] + r * a.sin()]
            })
            .collect();
        polys.push(poly);
    }
    for (i, w) in s.points.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            continue;
        }
        let (nx, ny) = (-dy / len, dx / len);
        let (ra, rb) = (radius(i), radius(i + 1));
        let mut quad = vec![
            [a[0] + nx * ra, a[1] + ny * ra],
            [b[0] + nx * rb, b[1] + ny * rb],
            [b[0] - nx * rb, b[1] - ny * rb],
            [a[0] - nx * ra, a[1] - ny * ra],
        ];
        if signed_area(&quad) < 0.0 {
            quad.reverse();
        }
        polys.push(quad);
    }
    polys
}

pub fn signed_area(poly: &[[f32; 2]]) -> f32 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5
}

fn real_array(doc: &PdfDocument, vals: &[f32]) -> Result<PdfObject, mupdf::Error> {
    let mut arr = doc.new_array_with_capacity(vals.len() as i32)?;
    for &v in vals {
        arr.array_push(PdfObject::new_real(v)?)?;
    }
    Ok(arr)
}

/// Append `stroke` to `page` as an ink annotation.
pub fn write_stroke(
    doc: &mut PdfDocument,
    page: &mut PdfPage,
    s: &Stroke,
) -> Result<(), mupdf::Error> {
    let pts = s.points.iter().map(|p| Point { x: p[0], y: p[1] });
    let mut a = page.add_ink_annotation([pts])?;
    a.set_color(AnnotationColor::Rgb {
        red: s.color[0] as f32 / 255.0,
        green: s.color[1] as f32 / 255.0,
        blue: s.color[2] as f32 / 255.0,
    })?;
    a.set_border_width(s.mean_width())?;
    a.set_author("mizu")?;
    let mut obj = a.object();
    obj.dict_put(
        "NM",
        PdfObject::new_string(&format!("{NM_PREFIX}{}", s.id))?,
    )?;
    obj.dict_put("MizuW", PdfObject::new_real(s.width)?)?;
    if let Some(p) = &s.pressure {
        obj.dict_put("MizuP", real_array(doc, p)?)?;
    }
    a.update()?;

    if s.pressure.is_some() {
        // Replace the generated appearance with the pressure shape.
        let inv = page
            .ctm()?
            .invert()
            .ok_or(mupdf::Error::NonInvertibleMatrix)?;
        let polys = stroke_polygons(s);
        let mut content = String::with_capacity(polys.len() * 120);
        content.push_str(&format!(
            "{:.4} {:.4} {:.4} rg\n",
            s.color[0] as f32 / 255.0,
            s.color[1] as f32 / 255.0,
            s.color[2] as f32 / 255.0
        ));
        let mut bb = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for poly in &polys {
            for (k, p) in poly.iter().enumerate() {
                let (x, y) = inv.transform_xy(p[0], p[1]);
                bb[0] = bb[0].min(x);
                bb[1] = bb[1].min(y);
                bb[2] = bb[2].max(x);
                bb[3] = bb[3].max(y);
                content.push_str(&format!(
                    "{x:.3} {y:.3} {}\n",
                    if k == 0 { "m" } else { "l" }
                ));
            }
            content.push_str("h\n");
        }
        content.push_str("f\n");
        let bbox = [bb[0] - 0.5, bb[1] - 0.5, bb[2] + 0.5, bb[3] + 0.5];
        let mut buf = Buffer::with_capacity(content.len());
        buf.write_all(content.as_bytes())
            .map_err(|_| mupdf::Error::InvalidUtf8)?;
        let mut dict = doc.new_dict()?;
        dict.dict_put("Type", PdfObject::new_name("XObject")?)?;
        dict.dict_put("Subtype", PdfObject::new_name("Form")?)?;
        dict.dict_put("BBox", real_array(doc, &bbox)?)?;
        dict.dict_put("Resources", doc.new_dict()?)?;
        let stream = doc.add_stream(&buf, Some(&dict), false)?;
        let mut ap = doc.new_dict()?;
        ap.dict_put("N", stream)?;
        let mut obj = a.object();
        obj.dict_put("Rect", real_array(doc, &bbox)?)?;
        obj.dict_put("AP", ap)?;
    }
    Ok(())
}

/// Write `src` plus `strokes` to `dst`, replacing any earlier mizu strokes.
/// `dst` is created atomically (temp file in the same directory, then rename).
pub fn save_with_strokes(
    src: &Path,
    dst: &Path,
    password: Option<&str>,
    strokes: &[Stroke],
) -> Result<(), String> {
    let mut doc = super::open_pdf(src, password).map_err(|e| e.to_string())?;
    let page_count = doc.page_count().map_err(|e| e.to_string())?.max(0) as usize;

    let mut by_page: Vec<Vec<&Stroke>> = vec![Vec::new(); page_count];
    for s in strokes {
        if s.page < page_count {
            by_page[s.page].push(s);
        }
    }
    for (i, list) in by_page.iter().enumerate() {
        let page = doc.load_page(i as i32).map_err(|e| e.to_string())?;
        let mut page = PdfPage::try_from(page).map_err(|e| e.to_string())?;
        let removed = remove_mizu(&mut page).map_err(|e| e.to_string())?;
        if list.is_empty() && removed == 0 {
            continue;
        }
        for s in list {
            write_stroke(&mut doc, &mut page, s).map_err(|e| format!("page {}: {e}", i + 1))?;
        }
    }

    let dir = dst
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let tmp = tempfile::Builder::new()
        .prefix(".mizu-save-")
        .suffix(".pdf")
        .tempfile_in(dir)
        .map_err(|e| format!("cannot write next to {}: {e}", dst.display()))?;
    let tmp_path = tmp
        .path()
        .to_str()
        .ok_or("path is not valid UTF-8")?
        .to_string();
    doc.save(&tmp_path).map_err(|e| e.to_string())?;
    // MuPDF keeps `src` open; Windows refuses to rename over an open file.
    drop(doc);
    if let Ok(f) = std::fs::File::open(tmp.path()) {
        let _ = f.sync_all();
    }
    if let Ok(meta) = std::fs::metadata(dst) {
        let _ = std::fs::set_permissions(tmp.path(), meta.permissions());
    }
    tmp.persist(dst).map_err(|e| e.error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygons_share_one_orientation() {
        let s = Stroke::new(
            0,
            vec![[0.0, 0.0], [10.0, 0.0], [20.0, 10.0], [5.0, 30.0]],
            Some(vec![0.2, 0.9, 0.5, 1.0]),
            3.0,
            [0; 3],
        );
        let polys = stroke_polygons(&s);
        assert!(polys.len() >= 4 + 3);
        for p in &polys {
            assert!(
                signed_area(p) > 0.0,
                "all polygons must be counter-clockwise"
            );
        }
    }

    #[test]
    fn polygon_radius_follows_pressure() {
        let s = Stroke::new(
            0,
            vec![[0.0, 0.0], [100.0, 0.0]],
            Some(vec![0.0, 1.0]),
            4.0,
            [0; 3],
        );
        let polys = stroke_polygons(&s);
        let extent = |p: &Vec<[f32; 2]>| {
            p.iter().map(|q| q[0]).fold(f32::MIN, f32::max)
                - p.iter().map(|q| q[0]).fold(f32::MAX, f32::min)
        };
        assert!(extent(&polys[0]) < extent(&polys[1]));
    }
}
