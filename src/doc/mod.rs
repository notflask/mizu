//! PDF access: opening, page metadata, outline, annotations and the
//! background threads that do the actual MuPDF work.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use mupdf::pdf::{PdfDocument, PdfPage};
use mupdf::{DestinationKind, Document};

use crate::ink::Stroke;

pub mod annots;
pub mod service;
pub mod worker;

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

#[derive(Clone, Debug)]
pub struct OutlineItem {
    pub title: String,
    pub page: Option<usize>,
    /// Target y in page space, when the destination names one.
    pub y: Option<f32>,
    pub level: u8,
}

#[derive(Debug)]
pub struct DocInfo {
    pub path: PathBuf,
    pub pages: Vec<PageMeta>,
    pub outline: Vec<OutlineItem>,
    pub strokes: Vec<Stroke>,
    pub password: Option<String>,
    pub mtime: Option<SystemTime>,
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

/// Open a PDF, authenticating when needed.
pub fn open_pdf(path: &Path, password: Option<&str>) -> Result<PdfDocument, OpenError> {
    let mut doc = Document::open(path).map_err(|e| OpenError::Failed(e.to_string()))?;
    if !doc.is_pdf() {
        return Err(OpenError::Failed("not a PDF document".into()));
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
pub fn load(path: &Path, password: Option<&str>) -> Result<DocInfo, OpenError> {
    let doc = open_pdf(path, password)?;
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
        if let Ok(pdf_page) = PdfPage::try_from(page) {
            strokes.extend(annots::read_strokes(i, &pdf_page));
        }
    }
    let mut outline = Vec::new();
    if let Ok(o) = doc.outlines() {
        flatten_outline(&o, 0, &mut outline);
    }
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    Ok(DocInfo {
        path: path.to_path_buf(),
        pages,
        outline,
        strokes,
        password: password.map(str::to_string),
        mtime,
    })
}
