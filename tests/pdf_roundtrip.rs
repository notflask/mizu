//! Integration tests that exercise real MuPDF (no window needed).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mizu::doc::annots;
use mizu::doc::service::{Job, LinkTarget, Reply, Service};
use mizu::doc::worker::{Pool, Rendered, TileKey, TILE};
use mizu::doc::{self, OpenError};
use mizu::ink::Stroke;
use mizu::testutil::{write_pdf, Extras, PageSpec};

fn sample(dir: &std::path::Path) -> std::path::PathBuf {
    write_pdf(
        dir,
        "sample.pdf",
        &[
            PageSpec::new(400.0, 600.0, "Hello Rust"),
            PageSpec::new(600.0, 400.0, "Second page hello"),
            PageSpec::new(400.0, 600.0, "Rotated page").rotated(90),
        ],
        &Extras {
            outline: true,
            links: true,
        },
    )
}

#[test]
fn loads_metadata_outline_and_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let info = doc::load(&path, None, Default::default()).unwrap();
    assert_eq!(info.pages.len(), 3);
    assert_eq!((info.pages[0].w, info.pages[0].h), (400.0, 600.0));
    assert_eq!((info.pages[1].w, info.pages[1].h), (600.0, 400.0));
    // /Rotate 90 swaps width and height in fitz page space.
    assert_eq!((info.pages[2].w, info.pages[2].h), (600.0, 400.0));
    assert_eq!(info.outline.len(), 3);
    assert_eq!(info.outline[1].page, Some(1));
    assert_eq!(info.outline[0].title, "Chapter 1");
    assert!(info.strokes.is_empty());
}

#[test]
fn missing_and_garbage_files_fail_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    assert!(doc::load(&dir.path().join("nope.pdf"), None, Default::default()).is_err());
    let junk = dir.path().join("junk.pdf");
    std::fs::write(&junk, b"definitely not a pdf").unwrap();
    assert!(matches!(
        doc::load(&junk, None, Default::default()),
        Err(OpenError::Failed(_))
    ));
}

fn wait_for<T>(rx: &crossbeam_channel::Receiver<T>, secs: u64) -> T {
    rx.recv_timeout(Duration::from_secs(secs))
        .expect("timed out waiting for worker")
}

#[test]
fn pool_renders_tiles_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let wakes = Arc::new(AtomicUsize::new(0));
    let w = wakes.clone();
    let pool = Pool::spawn(
        path,
        None,
        Arc::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }),
        2,
    );
    let scale = 2.0f32;
    let key = TileKey {
        page: 0,
        scale: scale.to_bits(),
        tx: 0,
        ty: 0,
    };
    pool.set_wanted(vec![key], vec![0]);
    let mut got_tile = false;
    let mut got_thumb = false;
    while !(got_tile && got_thumb) {
        match wait_for(&pool.rx, 20) {
            Rendered::Tile(t) => {
                got_tile = true;
                assert_eq!(t.key, key);
                assert_eq!(t.data.len(), (TILE * TILE * 4) as usize);
                assert_eq!((t.w as u32, t.h as u32), (TILE, TILE));
                // The page has a blue bar at y=20..50 (pdf space) -> near the
                // bottom of a 1200 px page, so look for any blue pixel in the
                // lower tiles; here just verify the tile is not blank white.
                let non_white = t
                    .data
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter(|p| p[0] < 250 || p[1] < 250)
                    .count();
                assert!(
                    non_white > 100,
                    "tile looks blank ({non_white} dark pixels)"
                );
                // alpha is opaque everywhere
                assert!(t.data.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
                pool.recycle(t.data);
            }
            Rendered::Thumb(t) => {
                got_thumb = true;
                assert_eq!(t.page, 0);
                assert_eq!(t.w.max(t.h) as u32, 256);
            }
            Rendered::Failed { error, .. } => panic!("render failed: {error}"),
            Rendered::OpenFailed(e) => panic!("open failed: {e}"),
        }
    }
    assert!(wakes.load(Ordering::SeqCst) >= 2);
}

#[test]
fn edge_tiles_report_valid_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let pool = Pool::spawn(path, None, Arc::new(|| {}), 1);
    // Page 0 is 400x600 pt; at scale 1.5 that is 600x900 px -> tile (1,1)
    // covers x 512..600 (88 px) and y 512..900 (388 px).
    let key = TileKey {
        page: 0,
        scale: 1.5f32.to_bits(),
        tx: 1,
        ty: 1,
    };
    pool.set_wanted(vec![key], vec![]);
    match wait_for(&pool.rx, 20) {
        Rendered::Tile(t) => assert_eq!((t.w, t.h), (88, 388)),
        _ => panic!("expected a tile"),
    }
}

fn pen(page: usize, y: f32, pressure: bool) -> Stroke {
    let pts: Vec<[f32; 2]> = (0..12)
        .map(|i| [50.0 + i as f32 * 10.0, y + (i as f32 * 0.7).sin() * 8.0])
        .collect();
    let pr = pressure.then(|| (0..12).map(|i| 0.2 + 0.07 * i as f32).collect());
    Stroke::new(page, pts, pr, 2.0, [0xe0, 0x31, 0x31])
}

#[test]
fn strokes_survive_save_and_load_including_rotated_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let strokes = vec![
        pen(0, 100.0, false),
        pen(1, 200.0, true),
        pen(2, 150.0, false),
        pen(2, 300.0, true),
    ];
    annots::save_with_strokes(&path, &path, None, &strokes).unwrap();

    let info = doc::load(&path, None, Default::default()).unwrap();
    assert_eq!(info.strokes.len(), 4);
    for orig in &strokes {
        let got = info
            .strokes
            .iter()
            .find(|s| s.id == orig.id)
            .unwrap_or_else(|| panic!("stroke {} missing", orig.id));
        assert_eq!(got.page, orig.page);
        assert_eq!(got.points.len(), orig.points.len());
        for (a, b) in got.points.iter().zip(&orig.points) {
            assert!(
                (a[0] - b[0]).abs() < 0.05 && (a[1] - b[1]).abs() < 0.05,
                "page {}: {a:?} vs {b:?}",
                orig.page
            );
        }
        assert_eq!(got.color, orig.color);
        assert!((got.width - orig.width).abs() < 1e-3);
        assert_eq!(got.pressure.is_some(), orig.pressure.is_some());
    }

    // Saving again with fewer strokes replaces the old ones (no duplicates).
    annots::save_with_strokes(&path, &path, None, &strokes[..1]).unwrap();
    let info = doc::load(&path, None, Default::default()).unwrap();
    assert_eq!(info.strokes.len(), 1);
    assert_eq!(info.strokes[0].id, strokes[0].id);

    // And an empty save removes everything.
    annots::save_with_strokes(&path, &path, None, &[]).unwrap();
    assert!(doc::load(&path, None, Default::default())
        .unwrap()
        .strokes
        .is_empty());
}

#[test]
fn save_to_other_path_leaves_source_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let before = std::fs::read(&path).unwrap();
    let copy = dir.path().join("copy.pdf");
    annots::save_with_strokes(&path, &copy, None, &[pen(0, 100.0, false)]).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        doc::load(&copy, None, Default::default())
            .unwrap()
            .strokes
            .len(),
        1
    );
}

#[test]
fn failed_save_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let before = std::fs::read(&path).unwrap();
    let bad = dir.path().join("no-such-dir").join("out.pdf");
    let err = annots::save_with_strokes(&path, &bad, None, &[pen(0, 100.0, false)]);
    assert!(err.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // No stray temp files next to the source.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".mizu-save-"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn mizu_strokes_are_not_rendered_by_mupdf_but_foreign_annots_are() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    // A huge, fully black stroke: if MuPDF drew it the tile would be black.
    let big = Stroke::new(
        0,
        vec![[0.0, 300.0], [400.0, 300.0]],
        None,
        400.0,
        [0, 0, 0],
    );
    annots::save_with_strokes(&path, &path, None, &[big]).unwrap();
    let pool = Pool::spawn(path, None, Arc::new(|| {}), 1);
    let key = TileKey {
        page: 0,
        scale: 1.0f32.to_bits(),
        tx: 0,
        ty: 0,
    };
    pool.set_wanted(vec![key], vec![]);
    match wait_for(&pool.rx, 20) {
        Rendered::Tile(t) => {
            let black = t
                .data
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] < 20 && p[1] < 20 && p[2] < 20)
                .count();
            // Only the text may be dark; a rendered 400pt-wide stroke would
            // cover > 100k pixels.
            assert!(
                black < 20_000,
                "mizu stroke leaked into the MuPDF render ({black})"
            );
        }
        _ => panic!("expected tile"),
    }
}

#[test]
fn search_finds_text_and_respects_case() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let svc = Service::spawn(path, None, Arc::new(|| {}));
    svc.send(Job::Search {
        id: 1,
        needle: "hello".into(),
        case_sensitive: false,
        start_page: 0,
        forward: true,
    });
    let mut pages = Vec::new();
    loop {
        match wait_for(&svc.rx, 20) {
            Reply::SearchPage { id: 1, page, hits } => {
                assert!(!hits.is_empty());
                pages.push(page);
            }
            Reply::SearchDone { id: 1 } => break,
            _ => {}
        }
    }
    assert_eq!(pages, vec![0, 1]);

    // "Hello" (capital H) only exists on page 1.
    svc.send(Job::Search {
        id: 2,
        needle: "Hello".into(),
        case_sensitive: true,
        start_page: 0,
        forward: true,
    });
    let mut pages = Vec::new();
    loop {
        match wait_for(&svc.rx, 20) {
            Reply::SearchPage { id: 2, page, .. } => pages.push(page),
            Reply::SearchDone { id: 2 } => break,
            _ => {}
        }
    }
    assert_eq!(pages, vec![0]);
}

#[test]
fn links_are_resolved() {
    let dir = tempfile::tempdir().unwrap();
    let path = sample(dir.path());
    let svc = Service::spawn(path, None, Arc::new(|| {}));
    svc.send(Job::Links { pages: vec![0] });
    match wait_for(&svc.rx, 20) {
        Reply::Links { page: 0, links } => {
            assert_eq!(links.len(), 2);
            assert!(links
                .iter()
                .any(|l| matches!(l.target, LinkTarget::Page { page: 1, .. })));
            assert!(links.iter().any(
                |l| matches!(&l.target, LinkTarget::Uri(u) if u.starts_with("https://example.org"))
            ));
            // Rect from the PDF is [20 20 120 60] in y-up space; on a 600 pt
            // tall page that is y 540..580 in page space.
            let l = links
                .iter()
                .find(|l| matches!(l.target, LinkTarget::Page { .. }))
                .unwrap();
            assert!(
                (l.rect[1] - 540.0).abs() < 1.0 && (l.rect[3] - 580.0).abs() < 1.0,
                "{:?}",
                l.rect
            );
        }
        _ => panic!("expected links"),
    }
}
