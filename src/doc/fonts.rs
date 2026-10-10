//! System fonts for MuPDF on Windows.
//!
//! MuPDF asks for a system font whenever a PDF uses a font it does not embed,
//! and for an EPUB once per styled element for every CSS font family that is
//! not built in (`serif`, say). The stock lookup (font-kit over DirectWrite)
//! loads every installed font to compare PostScript names, which takes
//! seconds, and MuPDF does not remember misses: a short book took minutes to
//! open. Here the installed fonts are indexed once with fontdb, so a lookup is
//! a scan over names in memory.
//!
//! Linux and macOS keep the stock lookup: fontconfig and Core Text are fast,
//! and fontconfig also resolves aliases such as `serif`.

use std::sync::{Once, OnceLock};

use fontdb::{Database, Family, Query, Style, Weight, ID};
use mupdf::{CjkFontOrdering, Font, FontHints, FontLoader};

/// Make MuPDF use this loader. Call before opening documents.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| mupdf::set_font_loader(SystemFonts));
}

pub struct SystemFonts;

fn db() -> &'static Database {
    static DB: OnceLock<Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = Database::new();
        db.load_system_fonts();
        log::debug!("{} system font faces", db.len());
        db
    })
}

/// The face MuPDF means by `name`: a PostScript name ("Arial-BoldMT"), or a
/// family name ("Georgia") matched by weight and slant.
fn find(db: &Database, name: &str, hints: FontHints) -> Option<ID> {
    if let Some(face) = db.faces().find(|f| f.post_script_name == name) {
        return Some(face.id);
    }
    let mut name = name;
    for suffix in ["MT", "PS", "IdentityH"] {
        name = name.strip_suffix(suffix).unwrap_or(name);
    }
    // DirectWrite matches family names regardless of case; fontdb does not.
    let family = db
        .faces()
        .flat_map(|f| &f.families)
        .find(|(f, _)| f.eq_ignore_ascii_case(name))?;
    db.query(&Query {
        families: &[Family::Name(&family.0)],
        weight: if hints.bold {
            Weight::BOLD
        } else {
            Weight::NORMAL
        },
        style: if hints.italic {
            Style::Italic
        } else {
            Style::Normal
        },
        ..Query::default()
    })
}

fn load(db: &Database, id: ID) -> Option<Font> {
    let family = db.face(id)?.families.first()?.0.clone();
    db.with_face_data(id, |data, index| {
        Font::from_bytes_with_index(&family, index as i32, data).ok()
    })?
}

impl FontLoader for SystemFonts {
    fn load_font(&self, name: &str, hints: FontHints) -> Option<Font> {
        let db = db();
        let font = load(db, find(db, name, hints)?)?;
        if hints.needs_exact_metrics
            && ((hints.bold && !font.is_bold()) || (hints.italic && !font.is_italic()))
        {
            return None;
        }
        Some(font)
    }

    fn load_cjk_font(&self, _name: &str, ordering: CjkFontOrdering, serif: bool) -> Option<Font> {
        // The fonts Windows ships for each ordering, as font-kit's loader
        // picked them.
        let names: &[&str] = match (ordering, serif) {
            (CjkFontOrdering::AdobeCns, true) => &["MingLiU"],
            (CjkFontOrdering::AdobeGb, true) => &["SimSun"],
            (CjkFontOrdering::AdobeJapan, true) => &["MS-Mincho"],
            (CjkFontOrdering::AdobeKorea, true) => &["Batang"],
            (CjkFontOrdering::AdobeCns, false) => &["DFKaiShu-SB-Estd-BF"],
            (CjkFontOrdering::AdobeGb, false) => &["KaiTi", "KaiTi_GB2312"],
            (CjkFontOrdering::AdobeJapan, false) => &["MS-Gothic"],
            (CjkFontOrdering::AdobeKorea, false) => &["Gulim"],
        };
        names
            .iter()
            .find_map(|n| self.load_font(n, FontHints::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_installed_fonts_by_postscript_and_family_name() {
        let db = db();
        // Machines without fonts (minimal containers) have nothing to find.
        let Some(face) = db.faces().find(|f| !f.post_script_name.is_empty()) else {
            return;
        };
        let hints = FontHints::default();
        assert_eq!(find(db, &face.post_script_name, hints), Some(face.id));
        let family = face.families[0].0.to_uppercase();
        assert!(find(db, &family, hints).is_some(), "{family}");
        assert!(SystemFonts
            .load_font(&face.post_script_name, hints)
            .is_some());
    }

    #[test]
    fn unknown_names_are_not_found() {
        let hints = FontHints::default();
        assert!(SystemFonts.load_font("NoSuchFont-Bold", hints).is_none());
    }
}
