//! EPUB books: opening, layout, search, outline, links, rendering, and the
//! read-only rules.

use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use mizu::config::Settings;
use mizu::doc::service::{search_page, LinkTarget};
use mizu::doc::worker::{Pool, Rendered, TileKey};
use mizu::doc::{self, Reflow};
use mizu::input::keys::parse_seq;
use mizu::testutil::write_epub;
use mizu::viewer::{LoadPurpose, UiMode, Viewer};

const TEXT: &str = "The quick brown fox jumps over the lazy dog, again and again, \
                    while the river keeps flowing past the old mill.";

fn book(dir: &std::path::Path) -> std::path::PathBuf {
    write_epub(
        dir,
        "book.epub",
        &[
            ("Beginning", TEXT),
            ("Middle", TEXT),
            ("Ending", "Here the zebra finally sleeps. Zebra."),
        ],
    )
}

#[test]
fn opens_and_lays_out() {
    let dir = tempfile::tempdir().unwrap();
    let path = book(dir.path());
    assert!(doc::is_epub(&path));
    let info = doc::load(&path, None, Reflow::default()).unwrap();
    assert!(info.reflowable);
    assert!(info.strokes.is_empty());
    let n = info.pages.len();
    assert!(n >= 3, "{n} pages");
    assert!((info.pages[0].w - Reflow::default().w).abs() < 1.0);
    // Smaller pages: more of them.
    let small = Reflow {
        h: 300.0,
        ..Reflow::default()
    };
    let more = doc::load(&path, None, small).unwrap().pages.len();
    assert!(more > n, "{more} > {n}");
    // Bigger text: more pages too.
    let big = Reflow {
        em: 20.0,
        ..Reflow::default()
    };
    assert!(doc::load(&path, None, big).unwrap().pages.len() > n);
}

#[test]
fn misnamed_epub_is_still_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let path = book(dir.path());
    let other = dir.path().join("book.bin");
    std::fs::copy(&path, &other).unwrap();
    assert!(doc::is_epub(&other));
    assert!(
        doc::load(&other, None, Reflow::default())
            .unwrap()
            .reflowable
    );
}

#[test]
fn outline_search_and_links() {
    let dir = tempfile::tempdir().unwrap();
    let path = book(dir.path());
    let info = doc::load(&path, None, Reflow::default()).unwrap();
    let titles: Vec<&str> = info.outline.iter().map(|o| o.title.as_str()).collect();
    assert_eq!(titles, vec!["Beginning", "Middle", "Ending"]);
    let last_chapter = info.outline[2].page.expect("target page");
    assert!(last_chapter > 0);

    let opened = doc::open_document(&path, None, Reflow::default()).unwrap();
    // The paragraph is repeated 12 times, with "zebra" and "Zebra" in it.
    let hits = search_page(opened.doc().unwrap(), last_chapter, "zebra", false);
    assert_eq!(hits.len(), 24);
    let hits = search_page(opened.doc().unwrap(), last_chapter, "Zebra", true);
    assert_eq!(hits.len(), 12);

    let svc = doc::service::Service::spawn_reflow(path, None, Reflow::default(), Arc::new(|| {}));
    svc.send(doc::service::Job::Links { pages: vec![0] });
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < end, "no links reply");
        if let Ok(doc::service::Reply::Links { links, .. }) =
            svc.rx.recv_timeout(Duration::from_millis(100))
        {
            let target = links.iter().find_map(|l| match l.target {
                LinkTarget::Page { page, .. } => Some(page),
                _ => None,
            });
            assert_eq!(target, Some(last_chapter));
            break;
        }
    }
}

#[test]
fn renders_tiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = book(dir.path());
    let pool = Pool::spawn_reflow(path, None, Reflow::default(), Arc::new(|| {}), 1);
    let key = TileKey {
        page: 0,
        scale: 1.0f32.to_bits(),
        tx: 0,
        ty: 0,
    };
    pool.set_wanted(vec![key], vec![]);
    match pool.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
        Rendered::Tile(t) => {
            let dark = t
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] < 128)
                .count();
            assert!(dark > 100, "text pixels: {dark}");
        }
        Rendered::Failed { error, .. } | Rendered::OpenFailed(error) => panic!("{error}"),
        Rendered::Thumb(_) => panic!("unexpected thumb"),
    }
}

fn isolate() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &dir);
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("XDG_STATE_HOME", dir.join("state"));
        std::env::set_var("XDG_DATA_HOME", dir.join("data"));
    });
}

fn pump(v: &mut Viewer, what: &str, mut done: impl FnMut(&mut Viewer) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        v.poll();
        v.tick(Instant::now());
        if done(v) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out waiting for: {what}");
}

fn keys(v: &mut Viewer, seq: &str) {
    for k in parse_seq(seq).expect("key sequence") {
        v.on_key(k);
    }
}

#[test]
fn viewer_is_read_only_and_relays_out() {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let path = book(dir.path());
    let mut v = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    v.set_window([800, 900], 1.0);
    v.open(path, LoadPurpose::Open { page: None });
    pump(&mut v, "book", |v| v.doc.is_some());
    assert!(v.is_reflowable());

    keys(&mut v, "i");
    assert_eq!(v.mode, UiMode::Normal);
    assert!(v.message.as_ref().unwrap().text.contains("only in PDFs"));
    keys(&mut v, ":w<CR>");
    assert!(v.message.as_ref().unwrap().text.contains("read-only"));
    assert!(!v.is_dirty());

    // Bigger text: more pages, and the reading position is kept.
    let before = v.page_count();
    keys(&mut v, "G");
    pump(&mut v, "scroll", |v| !v.tick(Instant::now()).animating);
    let frac = v.reading_fraction();
    keys(&mut v, ":fontsize 22<CR>");
    pump(&mut v, "relayout", |v| v.page_count() != before);
    assert!(v.page_count() > before);
    assert!(
        (v.reading_fraction() - frac).abs() < 0.1,
        "{} vs {frac}",
        v.reading_fraction()
    );
    // :wq quits without trying to write.
    keys(&mut v, ":wq<CR>");
    assert!(v.quit);
}

fn comic(dir: &std::path::Path, rtl: bool) -> std::path::PathBuf {
    let path = dir.join("comic.epub");
    // Cover, six pages, one double page in the middle.
    let mut sizes = vec![(300, 450); 8];
    sizes[4] = (600, 450);
    std::fs::write(&path, mizu::testutil::make_fxl_epub(&sizes, rtl)).unwrap();
    path
}

#[test]
fn fixed_layout_pages_come_from_their_images() {
    let dir = tempfile::tempdir().unwrap();
    let path = comic(dir.path(), true);
    let info = doc::load(&path, None, Reflow::default()).unwrap();
    let book = info.book.as_ref().unwrap();
    assert!(book.fixed && book.rtl);
    assert_eq!(book.title.as_deref(), Some("Comic & Co"));
    assert_eq!(info.pages.len(), 8);
    assert_eq!((info.pages[0].w, info.pages[0].h), (300.0, 450.0));
    assert_eq!(info.pages[4].w, 600.0);
    assert_eq!(book.sides[4], doc::SpreadSide::Center);
    let toc: Vec<(&str, Option<usize>)> = info
        .outline
        .iter()
        .map(|o| (o.title.as_str(), o.page))
        .collect();
    assert_eq!(toc, vec![("Chapter 1", Some(0)), ("Chapter 2", Some(2))]);

    // The tile is the page's image, not reflowed text.
    let pool = Pool::spawn_reflow(path, None, Reflow::default(), Arc::new(|| {}), 1);
    let key = TileKey {
        page: 1,
        scale: 1.0f32.to_bits(),
        tx: 0,
        ty: 0,
    };
    pool.set_wanted(vec![key], vec![]);
    match pool.rx.recv_timeout(Duration::from_secs(20)).unwrap() {
        Rendered::Tile(t) => {
            assert_eq!((t.w, t.h), (300, 450));
            let px = &t.data[(100 * 512 + 100) * 4..][..3];
            assert_eq!(px, &[60, 60, 60], "the grey of page 2");
        }
        Rendered::Failed { error, .. } | Rendered::OpenFailed(error) => panic!("{error}"),
        Rendered::Thumb(_) => panic!("unexpected thumb"),
    }
}

#[test]
fn manga_spreads_read_right_to_left() {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let mut v = Viewer::new(Settings::default(), None, Arc::new(|| {}));
    // A wide window: the book's "landscape" spreads are on.
    v.set_window([1400, 800], 1.0);
    v.open(comic(dir.path(), true), LoadPurpose::Open { page: None });
    pump(&mut v, "comic", |v| v.doc.is_some());
    assert!(v.spreads_active());
    let d = v.doc.as_ref().unwrap();
    let p = &d.layout.pages;
    // Cover alone, then pairs with the first page on the right.
    assert!(p[1].x > p[2].x && p[1].y == p[2].y);
    // The double page stands alone.
    assert!(p[4].y > p[3].y && p[4].y < p[5].y);
    // Read-only, no text to search.
    keys(&mut v, "i");
    assert_eq!(v.mode, UiMode::Normal);

    // J J: two rows on (cover -> 2|3 -> 4|5... here 4 is page index 3).
    keys(&mut v, "JJ");
    pump(&mut v, "pages", |v| !v.tick(Instant::now()).animating);
    assert_eq!(v.current_page(), 3);

    // Direction and spreads can be changed.
    keys(&mut v, ":direction ltr<CR>");
    let p = &v.doc.as_ref().unwrap().layout.pages;
    assert!(p[1].x < p[2].x);
    keys(&mut v, ":spread off<CR>");
    assert!(!v.spreads_active());
    let p = &v.doc.as_ref().unwrap().layout.pages;
    assert!(p[2].y > p[1].y);
}
