//! Tiny PDF generator for tests and benchmarks. Not part of the public API.

#[derive(Clone, Debug)]
pub struct PageSpec {
    pub w: f32,
    pub h: f32,
    pub rotate: i32,
    pub text: String,
}

impl PageSpec {
    pub fn new(w: f32, h: f32, text: &str) -> Self {
        PageSpec {
            w,
            h,
            rotate: 0,
            text: text.to_string(),
        }
    }

    pub fn rotated(mut self, degrees: i32) -> Self {
        self.rotate = degrees;
        self
    }
}

#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// Add an outline with one entry per page.
    pub outline: bool,
    /// Add a link on page 1 to page 2 and one to a URI.
    pub links: bool,
}

/// Build a small but valid PDF.
pub fn make_pdf(pages: &[PageSpec], extras: &Extras) -> Vec<u8> {
    let n = pages.len();
    // Object numbering: 1 catalog, 2 pages, 3 font, then per page: page, content.
    let first_page_obj = 4;
    let page_obj = |i: usize| first_page_obj + 2 * i;
    let content_obj = |i: usize| first_page_obj + 2 * i + 1;
    let mut next = first_page_obj + 2 * n;
    let outline_root = if extras.outline {
        let v = next;
        next += 1 + n;
        Some(v)
    } else {
        None
    };
    let link_objs = if extras.links {
        let v = next;
        next += 2;
        Some(v)
    } else {
        None
    };

    let mut objs: Vec<(usize, Vec<u8>)> = Vec::new();
    let kids: String = (0..n).map(|i| format!("{} 0 R ", page_obj(i))).collect();
    let mut catalog = "<< /Type /Catalog /Pages 2 0 R".to_string();
    if let Some(o) = outline_root {
        catalog.push_str(&format!(" /Outlines {o} 0 R"));
    }
    catalog.push_str(" >>");
    objs.push((1, catalog.into_bytes()));
    objs.push((
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes(),
    ));
    objs.push((
        3,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
    ));

    for (i, p) in pages.iter().enumerate() {
        let mut dict = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >>",
            p.w,
            p.h,
            content_obj(i)
        );
        if p.rotate != 0 {
            dict.push_str(&format!(" /Rotate {}", p.rotate));
        }
        if i == 0 {
            if let Some(l) = link_objs {
                dict.push_str(&format!(" /Annots [{} 0 R {} 0 R]", l, l + 1));
            }
        }
        dict.push_str(" >>");
        objs.push((page_obj(i), dict.into_bytes()));
        let content = format!(
            "0 0 1 rg 20 20 {} {} re f 0 g BT /F1 24 Tf 40 {} Td ({}) Tj ET",
            (p.w - 40.0).max(1.0),
            30.0,
            p.h - 60.0,
            p.text
        );
        objs.push((
            content_obj(i),
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                content
            )
            .into_bytes(),
        ));
    }
    if let Some(root) = outline_root {
        objs.push((
            root,
            format!(
                "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {n} >>",
                root + 1,
                root + n
            )
            .into_bytes(),
        ));
        for i in 0..n {
            let mut d = format!(
                "<< /Title (Chapter {}) /Parent {root} 0 R /Dest [{} 0 R /XYZ 0 {} null]",
                i + 1,
                page_obj(i),
                pages[i].h - 100.0
            );
            if i > 0 {
                d.push_str(&format!(" /Prev {} 0 R", root + i));
            }
            if i + 1 < n {
                d.push_str(&format!(" /Next {} 0 R", root + i + 2));
            }
            d.push_str(" >>");
            objs.push((root + 1 + i, d.into_bytes()));
        }
    }
    if let Some(l) = link_objs {
        let target = if n > 1 { page_obj(1) } else { page_obj(0) };
        objs.push((
            l,
            format!(
                "<< /Type /Annot /Subtype /Link /Rect [20 20 120 60] /Border [0 0 0] /Dest [{target} 0 R /Fit] >>"
            )
            .into_bytes(),
        ));
        objs.push((
            l + 1,
            b"<< /Type /Annot /Subtype /Link /Rect [200 20 300 60] /Border [0 0 0] /A << /S /URI /URI (https://example.org/) >> >>"
                .to_vec(),
        ));
    }

    objs.sort_by_key(|(n, _)| *n);
    let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = vec![0usize; next];
    for (num, body) in &objs {
        offsets[*num] = out.len();
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {next}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for off in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {next} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

/// Write a generated PDF into `dir` and return its path.
pub fn write_pdf(
    dir: &std::path::Path,
    name: &str,
    pages: &[PageSpec],
    extras: &Extras,
) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, make_pdf(pages, extras)).expect("write fixture");
    path
}
