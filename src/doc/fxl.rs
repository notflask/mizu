//! Fixed-layout EPUBs (`rendition:layout` = `pre-paginated`): comics and
//! manga, where every spine item is one page of a fixed size, usually a
//! single image. MuPDF only knows reflowable EPUBs, so these are read here:
//! the package document gives the pages, their sizes (the `viewport` meta),
//! the reading direction and spread hints, and the navigation document the
//! table of contents. Pages that are a single image are drawn from that
//! image directly; other fixed-layout pages fall back to MuPDF's layout.

use std::collections::HashMap;
use std::path::Path;

use super::zip::{resolve, Zip};
use super::OutlineItem;

/// Where a page wants to sit in a two-page spread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpreadSide {
    #[default]
    Auto,
    Left,
    Right,
    /// Alone, centred (`page-spread-center`).
    Center,
}

/// The book's own spread preference (`rendition:spread`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SpreadPref {
    None,
    Landscape,
    Both,
    #[default]
    Auto,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FixedPage {
    /// The XHTML of the page (zip entry).
    pub doc: String,
    /// The image the page consists of, if it is just one image.
    pub image: Option<String>,
    /// Page size in CSS pixels.
    pub w: f32,
    pub h: f32,
    pub side: SpreadSide,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FixedBook {
    pub title: Option<String>,
    pub pages: Vec<FixedPage>,
    /// `page-progression-direction="rtl"` (manga).
    pub rtl: bool,
    pub spread: SpreadPref,
    pub outline: Vec<OutlineItem>,
}

impl FixedBook {
    /// Every page is a single image (a comic).
    pub fn all_images(&self) -> bool {
        !self.pages.is_empty() && self.pages.iter().all(|p| p.image.is_some())
    }
}

// ---------------------------------------------------------------------------
// A tolerant tag scanner: enough for package and navigation documents.

#[derive(Debug)]
struct Tag<'a> {
    /// Local name, lower case, without namespace prefix.
    name: String,
    closing: bool,
    attrs: Vec<(String, String)>,
    /// Byte offset just after the tag.
    end: usize,
    _src: &'a str,
}

impl Tag<'_> {
    /// Attribute by local name (namespace prefixes ignored).
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name || k.rsplit(':').next() == Some(name))
            .map(|(_, v)| v.as_str())
    }

    /// The exact attribute name (with prefix), for `xlink:href` and friends.
    fn attr_exact(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

fn tags(src: &str) -> Vec<Tag<'_>> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while let Some(off) = src[i..].find('<') {
        let start = i + off;
        if src[start..].starts_with("<!--") {
            i = src[start..]
                .find("-->")
                .map(|e| start + e + 3)
                .unwrap_or(src.len());
            continue;
        }
        if src[start..].starts_with("<![CDATA[") {
            i = src[start..]
                .find("]]>")
                .map(|e| start + e + 3)
                .unwrap_or(src.len());
            continue;
        }
        // Find the closing '>' outside quotes.
        let mut j = start + 1;
        let mut quote = 0u8;
        while j < b.len() {
            let c = b[j];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
            } else if c == b'"' || c == b'\'' {
                quote = c;
            } else if c == b'>' {
                break;
            }
            j += 1;
        }
        let inner = &src[start + 1..j.min(src.len())];
        i = (j + 1).min(src.len());
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        let closing = inner.starts_with('/');
        let inner = inner.trim_start_matches('/').trim_end_matches('/');
        let name_end = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let raw_name = &inner[..name_end];
        let name = raw_name
            .rsplit(':')
            .next()
            .unwrap_or(raw_name)
            .to_ascii_lowercase();
        let attrs = parse_attrs(&inner[name_end..]);
        out.push(Tag {
            name,
            closing,
            attrs,
            end: i,
            _src: src,
        });
    }
    out
}

fn parse_attrs(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        let k0 = i;
        while i < b.len() && b[i] != b'=' && !(b[i] as char).is_whitespace() {
            i += 1;
        }
        let key = &s[k0..i];
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] != b'=' {
            if !key.is_empty() {
                out.push((key.to_string(), String::new()));
            }
            continue;
        }
        i += 1;
        while i < b.len() && (b[i] as char).is_whitespace() {
            i += 1;
        }
        let (v0, v1);
        if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
            let q = b[i];
            v0 = i + 1;
            i = v0;
            while i < b.len() && b[i] != q {
                i += 1;
            }
            v1 = i;
            i += 1;
        } else {
            v0 = i;
            while i < b.len() && !(b[i] as char).is_whitespace() {
                i += 1;
            }
            v1 = i;
        }
        out.push((key.to_string(), unescape(&s[v0..v1.min(s.len())])));
    }
    out
}

fn unescape(s: &str) -> String {
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
        let c = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => out.push(c),
            None => out.push_str(&rest[..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Text between the end of tag `i` and the next tag named `until` closing.
fn text_after(src: &str, all: &[Tag], i: usize, until: &str) -> String {
    let start = all[i].end;
    let end = all[i + 1..]
        .iter()
        .find(|t| t.closing && t.name == until)
        .map(|t| {
            // Position of that closing tag's '<'.
            src[..t.end].rfind('<').unwrap_or(t.end)
        })
        .unwrap_or(src.len());
    let raw = &src[start..end.max(start)];
    // Drop nested tags, collapse whitespace.
    let mut text = String::new();
    let mut in_tag = false;
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => text.push(c),
            _ => {}
        }
    }
    unescape(&text.split_whitespace().collect::<Vec<_>>().join(" "))
}

// ---------------------------------------------------------------------------

/// The package document's path, from `META-INF/container.xml`.
fn opf_path(zip: &Zip) -> Option<String> {
    let c = zip.read_string("META-INF/container.xml").ok()?;
    tags(&c)
        .into_iter()
        .find(|t| t.name == "rootfile" && !t.closing)
        .and_then(|t| t.attr("full-path").map(str::to_string))
}

/// `<meta name="viewport" content="width=1592, height=2475">`.
fn viewport(xhtml: &str) -> Option<(f32, f32)> {
    let all = tags(xhtml);
    let meta = all.iter().find(|t| {
        t.name == "meta" && t.attr("name").map(|n| n.eq_ignore_ascii_case("viewport")) == Some(true)
    })?;
    let content = meta.attr("content")?;
    let mut w = None;
    let mut h = None;
    for part in content.split([',', ';']) {
        let (k, v) = part.split_once('=')?;
        let v: Option<f32> = v.trim().trim_end_matches("px").parse().ok();
        match k.trim() {
            "width" => w = v,
            "height" => h = v,
            _ => {}
        }
    }
    Some((w?, h?)).filter(|(w, h)| *w > 0.0 && *h > 0.0)
}

/// SVG `viewBox` of the page, the other common way to give the size.
fn svg_viewbox(xhtml: &str) -> Option<(f32, f32)> {
    let all = tags(xhtml);
    let svg = all.iter().find(|t| t.name == "svg" && !t.closing)?;
    let vb: Vec<f32> = svg
        .attr("viewBox")?
        .split([' ', ','])
        .filter_map(|v| v.parse().ok())
        .collect();
    (vb.len() == 4 && vb[2] > 0.0 && vb[3] > 0.0).then_some((vb[2], vb[3]))
}

/// The single image a page shows, if the body has exactly one and no text.
fn single_image(xhtml: &str) -> Option<String> {
    let body_at = xhtml.find("<body").unwrap_or(0);
    let body = &xhtml[body_at..];
    let all = tags(body);
    let images: Vec<&Tag> = all
        .iter()
        .filter(|t| !t.closing && (t.name == "img" || t.name == "image"))
        .collect();
    if images.len() != 1 {
        return None;
    }
    // Visible text besides the image means it is not just a picture.
    let mut text = String::new();
    let mut in_tag = false;
    for c in body.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag && !c.is_whitespace() => text.push(c),
            _ => {}
        }
    }
    if text.chars().count() > 3 {
        return None;
    }
    let t = images[0];
    t.attr("src")
        .or_else(|| t.attr_exact("xlink:href"))
        .or_else(|| t.attr("href"))
        .map(str::to_string)
}

/// Image size from the PNG / JPEG / GIF / WebP header, for pages without a
/// viewport.
pub fn image_size(data: &[u8]) -> Option<(f32, f32)> {
    let be32 = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
    };
    let be16 = |at: usize| -> Option<u16> {
        Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?))
    };
    if data.starts_with(b"\x89PNG") {
        return Some((be32(16)? as f32, be32(20)? as f32));
    }
    if data.starts_with(b"GIF8") {
        let w = u16::from_le_bytes(data.get(6..8)?.try_into().ok()?);
        let h = u16::from_le_bytes(data.get(8..10)?.try_into().ok()?);
        return Some((w as f32, h as f32));
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let chunk = data.get(12..16)?;
        let le24 = |at: usize| -> Option<u32> {
            let b = data.get(at..at + 3)?;
            Some(b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16)
        };
        return match chunk {
            b"VP8X" => Some(((le24(24)? + 1) as f32, (le24(27)? + 1) as f32)),
            b"VP8 " => {
                let w = u16::from_le_bytes(data.get(26..28)?.try_into().ok()?) & 0x3fff;
                let h = u16::from_le_bytes(data.get(28..30)?.try_into().ok()?) & 0x3fff;
                Some((w as f32, h as f32))
            }
            b"VP8L" => {
                let b = data.get(21..25)?;
                let v = u32::from_le_bytes(b.try_into().ok()?);
                Some((((v & 0x3fff) + 1) as f32, (((v >> 14) & 0x3fff) + 1) as f32))
            }
            _ => None,
        };
    }
    if data.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 9 < data.len() {
            if data[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = data[i + 1];
            let len = be16(i + 2)? as usize;
            // SOF0..SOF15 except DHT (C4), JPG (C8), DAC (CC).
            if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                return Some((be16(i + 7)? as f32, be16(i + 5)? as f32));
            }
            i += 2 + len;
        }
    }
    None
}

/// Read the fixed-layout structure of an EPUB. `None` when the book is
/// reflowable (or not readable this way); MuPDF handles those.
pub fn read(path: &Path) -> Option<FixedBook> {
    let zip = Zip::open(path).ok()?;
    read_from(&zip)
}

pub fn read_from(zip: &Zip) -> Option<FixedBook> {
    let opf_name = opf_path(zip)?;
    let opf = zip.read_string(&opf_name).ok()?;
    let all = tags(&opf);

    let mut global_fixed = false;
    let mut spread = SpreadPref::Auto;
    let mut title = None;
    let mut writing_rl = false;
    for (i, t) in all.iter().enumerate() {
        if t.closing {
            continue;
        }
        match t.name.as_str() {
            "meta" => {
                if let Some(prop) = t.attr("property") {
                    let value = text_after(&opf, &all, i, "meta");
                    match prop {
                        "rendition:layout" => global_fixed = value == "pre-paginated",
                        "rendition:spread" => {
                            spread = match value.as_str() {
                                "none" => SpreadPref::None,
                                "landscape" => SpreadPref::Landscape,
                                "both" | "portrait" => SpreadPref::Both,
                                _ => SpreadPref::Auto,
                            }
                        }
                        _ => {}
                    }
                }
                if t.attr("name") == Some("primary-writing-mode") {
                    writing_rl = t
                        .attr("content")
                        .map(|c| c.ends_with("-rl"))
                        .unwrap_or(false);
                }
                // Old EPUB 2 comics: <meta name="fixed-layout" content="true"/>.
                if t.attr("name") == Some("fixed-layout") && t.attr("content") == Some("true") {
                    global_fixed = true;
                }
            }
            "title" if title.is_none() => {
                let v = text_after(&opf, &all, i, "title");
                if !v.is_empty() {
                    title = Some(v);
                }
            }
            _ => {}
        }
    }
    // Apple's display options also mark fixed-layout books.
    if !global_fixed {
        if let Ok(d) = zip.read_string("META-INF/com.apple.ibooks.display-options.xml") {
            global_fixed = d.contains("\"fixed-layout\">true");
        }
    }

    let mut manifest: HashMap<String, (String, String, String)> = HashMap::new();
    let mut nav_href = None;
    let mut ncx_href = None;
    for t in all.iter().filter(|t| !t.closing && t.name == "item") {
        let (Some(id), Some(href)) = (t.attr("id"), t.attr("href")) else {
            continue;
        };
        let full = resolve(&opf_name, href);
        let props = t.attr("properties").unwrap_or("").to_string();
        let media = t.attr("media-type").unwrap_or("").to_string();
        if props.split_whitespace().any(|p| p == "nav") {
            nav_href = Some(full.clone());
        }
        if media == "application/x-dtbncx+xml" {
            ncx_href = Some(full.clone());
        }
        manifest.insert(id.to_string(), (full, media, props));
    }

    let spine = all.iter().find(|t| !t.closing && t.name == "spine")?;
    let rtl = match spine.attr("page-progression-direction") {
        Some("rtl") => true,
        Some("ltr") => false,
        _ => writing_rl,
    };
    if ncx_href.is_none() {
        if let Some(toc) = spine.attr("toc") {
            ncx_href = manifest.get(toc).map(|m| m.0.clone());
        }
    }

    let mut pages = Vec::new();
    let mut any_fixed = global_fixed;
    for t in all.iter().filter(|t| !t.closing && t.name == "itemref") {
        if t.attr("linear") == Some("no") {
            continue;
        }
        let Some((doc, media, _)) = t.attr("idref").and_then(|id| manifest.get(id)) else {
            continue;
        };
        let props = t.attr("properties").unwrap_or("");
        let mut fixed = global_fixed;
        let mut side = SpreadSide::Auto;
        for p in props.split_whitespace() {
            match p {
                "rendition:layout-pre-paginated" => fixed = true,
                "rendition:layout-reflowable" => fixed = false,
                "page-spread-left" | "rendition:page-spread-left" => side = SpreadSide::Left,
                "page-spread-right" | "rendition:page-spread-right" => side = SpreadSide::Right,
                "rendition:page-spread-center" | "page-spread-center" => side = SpreadSide::Center,
                _ => {}
            }
        }
        if !fixed {
            // A mixed book: mizu reads it as a reflowable one.
            return None;
        }
        any_fixed = true;
        let (image, size) = if media.starts_with("image/") {
            // A spine item that is an image itself.
            let size = zip.read(doc).ok().and_then(|d| image_size(&d));
            (Some(doc.clone()), size)
        } else {
            let xhtml = zip.read_string(doc).ok()?;
            let image = single_image(&xhtml).map(|src| resolve(doc, &src));
            let image = image.filter(|i| zip.contains(i));
            let mut size = viewport(&xhtml).or_else(|| svg_viewbox(&xhtml));
            if size.is_none() {
                if let Some(i) = &image {
                    size = zip.read(i).ok().and_then(|d| image_size(&d));
                }
            }
            (image, size)
        };
        let (w, h) = size.unwrap_or((1200.0, 1800.0));
        pages.push(FixedPage {
            doc: doc.clone(),
            image,
            w,
            h,
            side,
        });
    }
    if !any_fixed || pages.is_empty() {
        return None;
    }

    let index: HashMap<&str, usize> = pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.doc.as_str(), i))
        .collect();
    let outline = nav_href
        .and_then(|n| read_nav(zip, &n, &index))
        .filter(|o| !o.is_empty())
        .or_else(|| ncx_href.and_then(|n| read_ncx(zip, &n, &index)))
        .unwrap_or_default();

    Some(FixedBook {
        title,
        pages,
        rtl,
        spread,
        outline,
    })
}

/// EPUB 3 navigation document: the `toc` nav.
fn read_nav(zip: &Zip, name: &str, index: &HashMap<&str, usize>) -> Option<Vec<OutlineItem>> {
    let src = zip.read_string(name).ok()?;
    let all = tags(&src);
    let mut in_toc = false;
    let mut nav_depth = 0;
    let mut level: i32 = -1;
    let mut out = Vec::new();
    for (i, t) in all.iter().enumerate() {
        match (t.name.as_str(), t.closing) {
            ("nav", false) => {
                nav_depth += 1;
                if t.attr("type")
                    .map(|v| v.split_whitespace().any(|x| x == "toc"))
                    == Some(true)
                {
                    in_toc = true;
                    level = -1;
                }
            }
            ("nav", true) => {
                nav_depth -= 1;
                if nav_depth == 0 {
                    in_toc = false;
                }
            }
            ("ol", false) if in_toc => level += 1,
            ("ol", true) if in_toc => level -= 1,
            ("a", false) if in_toc => {
                let title = text_after(&src, &all, i, "a");
                let page = t
                    .attr("href")
                    .map(|h| resolve(name, h))
                    .and_then(|h| index.get(h.as_str()).copied());
                out.push(OutlineItem {
                    title,
                    page,
                    y: None,
                    level: level.max(0) as u8,
                });
            }
            _ => {}
        }
    }
    Some(out)
}

/// EPUB 2 NCX table of contents.
fn read_ncx(zip: &Zip, name: &str, index: &HashMap<&str, usize>) -> Option<Vec<OutlineItem>> {
    let src = zip.read_string(name).ok()?;
    let all = tags(&src);
    let mut level: i32 = -1;
    let mut title = String::new();
    let mut out = Vec::new();
    for (i, t) in all.iter().enumerate() {
        match (t.name.as_str(), t.closing) {
            ("navpoint", false) => level += 1,
            ("navpoint", true) => level -= 1,
            ("text", false) => title = text_after(&src, &all, i, "text"),
            ("content", false) => {
                let page = t
                    .attr("src")
                    .map(|h| resolve(name, h))
                    .and_then(|h| index.get(h.as_str()).copied());
                out.push(OutlineItem {
                    title: std::mem::take(&mut title),
                    page,
                    y: None,
                    level: level.max(0) as u8,
                });
            }
            _ => {}
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_tags_and_attributes() {
        let t =
            tags(r#"<?xml?><!-- <x> --><a href='b.html' title="x &amp; y">T</a><img src=c.png/>"#);
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].attr("href"), Some("b.html"));
        assert_eq!(t[0].attr("title"), Some("x & y"));
        assert!(t[1].closing);
        assert_eq!(t[2].attr("src"), Some("c.png"));
    }

    #[test]
    fn viewport_and_images() {
        let x = r#"<html><head><meta name="viewport" content="width=1592, height=2475"/></head>
            <body><img src="../images/p1.png" alt=""/></body></html>"#;
        assert_eq!(viewport(x), Some((1592.0, 2475.0)));
        assert_eq!(single_image(x).as_deref(), Some("../images/p1.png"));
        let svg = r#"<body><svg xmlns:xlink="x" viewBox="0 0 800 1200"><image xlink:href="i.jpg"/></svg></body>"#;
        assert_eq!(svg_viewbox(svg), Some((800.0, 1200.0)));
        assert_eq!(single_image(svg).as_deref(), Some("i.jpg"));
        let text = "<body><p>Some real text here</p><img src='a.png'/></body>";
        assert_eq!(single_image(text), None);
    }

    #[test]
    fn image_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640.0, 480.0)));
        let jpg = [
            0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 11, 8, 1, 0x2c, 2, 0x58, 3, 0, 0,
        ];
        assert_eq!(image_size(&jpg), Some((600.0, 300.0)));
    }
}
