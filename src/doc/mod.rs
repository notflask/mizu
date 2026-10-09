//! Document access (PDF and EPUB): opening, page metadata, outline,
//! annotations and the background threads that do the actual MuPDF work.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mupdf::pdf::{PdfDocument, PdfPage};
use mupdf::{DestinationKind, Document};

use crate::ink::Stroke;

pub mod annots;
pub mod fxl;
pub mod service;
pub mod worker;
pub mod zip;

pub use fxl::{SpreadPref, SpreadSide};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageMeta {
    pub w: f32,
    pub h: f32,
    /// Top-left corner of the page bounds in MuPDF's page space (normally 0,0).
    pub x0: f32,
    pub y0: f32,
}

impl PageMeta {
    pub const FALLBACK: PageMeta = PageMeta {
        w: 595.0,
        h: 842.0,
        x0: 0.0,
        y0: 0.0,
    };
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<usize>,
    /// Target y in page space, when the destination names one.
    pub y: Option<f32>,
    pub level: u8,
}

/// The virtual page reflowable documents (EPUB) are laid out to, in points,
/// and their text size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflow {
    pub w: f32,
    pub h: f32,
    pub em: f32,
}

impl Default for Reflow {
    fn default() -> Self {
        Reflow {
            w: 480.0,
            h: 680.0,
            em: 11.0,
        }
    }
}

/// An open document: a PDF (editable, with ink), a reflowable book laid
/// out by MuPDF, or a fixed-layout book of page images (comics, manga).
/// The last two are read-only.
pub enum OpenDoc {
    Pdf(PdfDocument),
    Reflow(Document),
    Fixed(FixedDoc),
}

pub struct FixedDoc {
    pub zip: zip::Zip,
    pub book: fxl::FixedBook,
}

impl OpenDoc {
    /// The MuPDF document, for everything but image books.
    pub fn doc(&self) -> Option<&Document> {
        match self {
            OpenDoc::Pdf(d) => Some(d),
            OpenDoc::Reflow(d) => Some(d),
            OpenDoc::Fixed(_) => None,
        }
    }

    pub fn is_pdf(&self) -> bool {
        matches!(self, OpenDoc::Pdf(_))
    }
}

/// What a book says about how to show it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BookInfo {
    pub title: Option<String>,
    /// Pages go right to left (manga).
    pub rtl: bool,
    pub spread: SpreadPref,
    /// Per page.
    pub sides: Vec<SpreadSide>,
    /// Fixed layout (pages are pictures of a fixed size).
    pub fixed: bool,
}

#[derive(Debug)]
pub struct DocInfo {
    pub path: PathBuf,
    pub pages: Vec<PageMeta>,
    pub outline: Vec<OutlineItem>,
    pub strokes: Vec<Stroke>,
    pub password: Option<String>,
    pub mtime: Option<SystemTime>,
    /// EPUB and friends: read-only.
    pub reflowable: bool,
    /// Present for EPUBs.
    pub book: Option<BookInfo>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum OpenError {
    #[error("password required")]
    NeedsPassword,
    #[error("wrong password")]
    WrongPassword,
    #[error("{0}")]
    Failed(String),
}

impl From<mupdf::Error> for OpenError {
    fn from(e: mupdf::Error) -> Self {
        OpenError::Failed(e.to_string())
    }
}

/// True for an EPUB file: a zip whose first entry is `mimetype` with
/// `application/epub+zip` (the format requires that), or by extension.
pub fn is_epub(path: &Path) -> bool {
    let by_ext = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("epub"))
        .unwrap_or(false);
    let mut head = [0u8; 58];
    let by_magic = std::fs::File::open(path)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .map(|_| {
            head.starts_with(b"PK\x03\x04") && &head[30..58] == b"mimetypeapplication/epub+zip"
        })
        .unwrap_or(false);
    by_ext || by_magic
}

fn open_raw(path: &Path, epub: bool) -> Result<Document, OpenError> {
    // MuPDF picks the format from the name. A misnamed EPUB is opened
    // from memory with the right type.
    if epub
        && !path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("epub"))
            .unwrap_or(false)
    {
        let bytes = std::fs::read(path).map_err(|e| OpenError::Failed(e.to_string()))?;
        return Document::from_bytes(&bytes, "application/epub+zip")
            .map_err(|e| OpenError::Failed(e.to_string()));
    }
    // MuPDF takes raw bytes on Unix (any file name works) and UTF-8 on Windows.
    #[cfg(unix)]
    let opened = Document::open(path);
    #[cfg(not(unix))]
    let opened = Document::open(
        path.to_str()
            .ok_or_else(|| OpenError::Failed("the file name is not valid UTF-8".into()))?,
    );
    opened.map_err(|e| OpenError::Failed(e.to_string()))
}

/// Open a PDF or an EPUB. EPUBs are laid out with `reflow`.
pub fn open_document(
    path: &Path,
    password: Option<&str>,
    reflow: Reflow,
) -> Result<OpenDoc, OpenError> {
    if is_epub(path) {
        let fixed = zip::Zip::open(path)
            .ok()
            .and_then(|z| fxl::read_from(&z).map(|b| (z, b)));
        let mut reflow = reflow;
        match fixed {
            Some((zip, book)) if book.all_images() => {
                return Ok(OpenDoc::Fixed(FixedDoc { zip, book }));
            }
            Some((_, book)) => {
                // Fixed-layout pages that are not just pictures: let MuPDF
                // lay them out at the size the book asks for.
                let p = &book.pages[0];
                reflow.w = p.w;
                reflow.h = p.h;
            }
            None => {}
        }
        let mut doc = open_raw(path, true)?;
        doc.layout(reflow.w, reflow.h, reflow.em)?;
        return Ok(OpenDoc::Reflow(doc));
    }
    open_pdf(path, password).map(OpenDoc::Pdf)
}

/// Open a PDF, authenticating when needed.
pub fn open_pdf(path: &Path, password: Option<&str>) -> Result<PdfDocument, OpenError> {
    let mut doc = open_raw(path, false)?;
    if !doc.is_pdf() {
        return Err(OpenError::Failed("not a PDF or EPUB document".into()));
    }
    if doc.needs_password()? {
        match password {
            None => return Err(OpenError::NeedsPassword),
            Some(p) => {
                if !doc.authenticate(p)? {
                    return Err(OpenError::WrongPassword);
                }
            }
        }
    }
    PdfDocument::try_from(doc).map_err(OpenError::from)
}

fn flatten_outline(items: &[mupdf::Outline], level: u8, out: &mut Vec<OutlineItem>) {
    for o in items {
        let (page, y) = match &o.dest {
            Some(d) => {
                let y = match d.kind {
                    DestinationKind::XYZ { top, .. } | DestinationKind::FitH { top } => top,
                    _ => None,
                };
                (Some(d.loc.page_number as usize), y)
            }
            None => (None, None),
        };
        out.push(OutlineItem {
            title: o.title.trim().to_string(),
            page,
            y,
            level,
        });
        flatten_outline(&o.down, level.saturating_add(1), out);
    }
}

/// Read everything the UI needs to lay out the document.
pub fn load(path: &Path, password: Option<&str>, reflow: Reflow) -> Result<DocInfo, OpenError> {
    let opened = open_document(path, password, reflow)?;
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    if let OpenDoc::Fixed(f) = &opened {
        let b = &f.book;
        return Ok(DocInfo {
            path: path.to_path_buf(),
            pages: b
                .pages
                .iter()
                .map(|p| PageMeta {
                    w: p.w,
                    h: p.h,
                    x0: 0.0,
                    y0: 0.0,
                })
                .collect(),
            outline: b.outline.clone(),
            strokes: Vec::new(),
            password: None,
            mtime,
            reflowable: true,
            book: Some(BookInfo {
                title: b.title.clone(),
                rtl: b.rtl,
                spread: b.spread,
                sides: b.pages.iter().map(|p| p.side).collect(),
                fixed: true,
            }),
        });
    }
    let doc = opened
        .doc()
        .ok_or_else(|| OpenError::Failed("no document".into()))?;
    let count = doc.page_count()?.max(0) as usize;
    let mut pages = Vec::with_capacity(count);
    let mut strokes = Vec::new();
    for i in 0..count {
        let page = match doc.load_page(i as i32) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("page {i}: {e}");
                pages.push(PageMeta::FALLBACK);
                continue;
            }
        };
        match page.bounds() {
            Ok(b) if b.width() > 0.0 && b.height() > 0.0 => pages.push(PageMeta {
                w: b.width(),
                h: b.height(),
                x0: b.x0,
                y0: b.y0,
            }),
            _ => pages.push(PageMeta::FALLBACK),
        }
        if opened.is_pdf() {
            if let Ok(pdf_page) = PdfPage::try_from(page) {
                strokes.extend(annots::read_strokes(i, &pdf_page));
            }
        }
    }
    let mut outline = Vec::new();
    if let Ok(o) = doc.outlines() {
        flatten_outline(&o, 0, &mut outline);
    }
    let book = (!opened.is_pdf()).then(|| {
        // A reflowable book still has a direction and a title.
        let fixed = fxl::read(path);
        BookInfo {
            title: fixed.as_ref().and_then(|b| b.title.clone()),
            rtl: fixed.as_ref().map(|b| b.rtl).unwrap_or(false),
            spread: SpreadPref::None,
            sides: vec![SpreadSide::Auto; count],
            fixed: fixed.is_some(),
        }
    });
    Ok(DocInfo {
        path: path.to_path_buf(),
        pages,
        outline,
        strokes,
        password: password.map(str::to_string),
        mtime,
        reflowable: !opened.is_pdf(),
        book,
    })
}
