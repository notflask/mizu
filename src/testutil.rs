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
        for (i, pg) in pages.iter().enumerate() {
            let mut d = format!(
                "<< /Title (Chapter {}) /Parent {root} 0 R /Dest [{} 0 R /XYZ 0 {} null]",
                i + 1,
                page_obj(i),
                pg.h - 100.0
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

/// A small EPUB 3 book: `chapters` are `(title, paragraph text)`. Chapter 1
/// links to the last chapter. Written as a zip without compression.
pub fn make_epub(chapters: &[(&str, &str)]) -> Vec<u8> {
    let mut files: Vec<(String, String)> = Vec::new();
    files.push(("mimetype".into(), "application/epub+zip".into()));
    files.push((
        "META-INF/container.xml".into(),
        r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#
            .into(),
    ));
    let mut manifest = String::new();
    let mut spine = String::new();
    let mut nav = String::new();
    let last = chapters.len();
    for (i, (title, text)) in chapters.iter().enumerate() {
        let n = i + 1;
        manifest.push_str(&format!(
            r#"<item id="c{n}" href="c{n}.xhtml" media-type="application/xhtml+xml"/>"#
        ));
        spine.push_str(&format!(r#"<itemref idref="c{n}"/>"#));
        nav.push_str(&format!(r#"<li><a href="c{n}.xhtml">{title}</a></li>"#));
        let link = if n == 1 && last > 1 {
            format!(r#"<p><a href="c{last}.xhtml">Go to the end</a></p>"#)
        } else {
            String::new()
        };
        // Repeat the text so chapters span a few pages.
        let body: String = (0..12).map(|_| format!("<p>{text}</p>")).collect();
        files.push((
            format!("OEBPS/c{n}.xhtml"),
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>{title}</title></head>
<body><h1>{title}</h1>{link}{body}</body></html>"#
            ),
        ));
    }
    files.push((
        "OEBPS/nav.xhtml".into(),
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><head><title>Contents</title></head>
<body><nav epub:type="toc"><ol>{nav}</ol></nav></body></html>"#
        ),
    ));
    files.push((
        "OEBPS/book.opf".into(),
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="id">mizu-test</dc:identifier><dc:title>Test</dc:title><dc:language>en</dc:language>
  </metadata>
  <manifest><item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>{manifest}</manifest>
  <spine>{spine}</spine>
</package>"#
        ),
    ));
    stored_zip(&files)
}

/// Write a generated EPUB into `dir` and return its path.
pub fn write_epub(
    dir: &std::path::Path,
    name: &str,
    chapters: &[(&str, &str)],
) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, make_epub(chapters)).expect("write fixture");
    path
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A zip archive with every file stored (no compression).
fn stored_zip(files: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let u16le = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let u32le = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let (name, data) = (name.as_bytes(), data.as_bytes());
        let crc = crc32(data);
        let offset = out.len() as u32;
        u32le(&mut out, 0x0403_4b50);
        for x in [20u16, 0, 0, 0, 0] {
            u16le(&mut out, x); // version, flags, method, time, date
        }
        u32le(&mut out, crc);
        u32le(&mut out, data.len() as u32);
        u32le(&mut out, data.len() as u32);
        u16le(&mut out, name.len() as u16);
        u16le(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        u32le(&mut central, 0x0201_4b50);
        for x in [20u16, 20, 0, 0, 0, 0] {
            u16le(&mut central, x); // made by, needed, flags, method, time, date
        }
        u32le(&mut central, crc);
        u32le(&mut central, data.len() as u32);
        u32le(&mut central, data.len() as u32);
        u16le(&mut central, name.len() as u16);
        for x in [0u16, 0, 0, 0] {
            u16le(&mut central, x); // extra, comment, disk, internal attrs
        }
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend_from_slice(name);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    u32le(&mut out, 0x0605_4b50);
    for x in [0u16, 0, files.len() as u16, files.len() as u16] {
        u16le(&mut out, x);
    }
    u32le(&mut out, central.len() as u32);
    u32le(&mut out, cd_offset);
    u16le(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc32_matches_the_reference() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
