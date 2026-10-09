//! Micro benchmarks for the hot paths. `cargo bench --profile fast`.

use std::hint::black_box;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};

use mizu::doc::annots;
use mizu::doc::worker::{Pool, Rendered, TileKey};
use mizu::ink::eraser;
use mizu::ink::{History, Store, Stroke};
use mizu::input::keymap::{KeyEngine, Keymaps, Mode};
use mizu::input::keys::parse_seq;
use mizu::render::recolor::{recolor, to_linear3, DarkTheme};
use mizu::render::tiles::wanted_tiles;
use mizu::testutil::{write_pdf, Extras, PageSpec};
use mizu::view::{Camera, Layout, ZoomMode};

fn wavy(page: usize, y: f32, n: usize) -> Stroke {
    let pts: Vec<[f32; 2]> = (0..n)
        .map(|i| [20.0 + i as f32 * 2.0, y + (i as f32 * 0.3).sin() * 6.0])
        .collect();
    Stroke::new(page, pts, None, 1.5, [30, 30, 30])
}

fn bench_tiles(c: &mut Criterion) {
    // A page with a lot of text so rendering costs something real.
    let dir = tempfile::tempdir().unwrap();
    let text = "The quick brown fox jumps over the lazy dog ".repeat(2);
    let pages: Vec<PageSpec> = (0..4).map(|_| PageSpec::new(595.0, 842.0, &text)).collect();
    let path = write_pdf(dir.path(), "bench.pdf", &pages, &Extras::default());
    let pool = Pool::spawn(path, None, Arc::new(|| {}), 1);

    // Warm the display-list cache.
    let warm = TileKey {
        page: 0,
        scale: 1.0f32.to_bits(),
        tx: 0,
        ty: 0,
    };
    pool.set_wanted(vec![warm], vec![]);
    let _ = pool.rx.recv_timeout(Duration::from_secs(10));

    let mut n = 0u32;
    c.bench_function("tile: 512x512 render + hand-off (hot display list)", |b| {
        b.iter(|| {
            n += 1;
            // A new scale each time defeats any result reuse.
            let key = TileKey {
                page: 0,
                scale: (1.0 + (n % 1000) as f32 * 1e-4).to_bits(),
                tx: 0,
                ty: 0,
            };
            pool.set_wanted(vec![key], vec![]);
            match pool.rx.recv_timeout(Duration::from_secs(10)) {
                Ok(Rendered::Tile(t)) => {
                    black_box(t.data.len());
                    pool.recycle(t.data);
                }
                _ => panic!("no tile"),
            }
        })
    });
}

fn bench_view(c: &mut Criterion) {
    let sizes: Vec<(f32, f32)> = (0..300).map(|_| (595.0, 842.0)).collect();
    c.bench_function("layout: 300 pages", |b| {
        b.iter(|| Layout::new(black_box(&sizes)))
    });
    let layout = Layout::new(&sizes);
    c.bench_function("layout: page_at_y", |b| {
        let mut y = 0.0f32;
        b.iter(|| {
            y = (y + 1234.5) % layout.height;
            black_box(layout.page_at_y(y))
        })
    });
    let cam = Camera {
        offset: [0.0, 100_000.0],
        zoom: 2.0,
        dpr: 1.0,
        viewport: [1920.0, 1080.0],
        mode: ZoomMode::Free,
    };
    let [_, y0, _, y1] = cam.visible_doc_rect();
    let vis = layout.visible(y0, y1);
    c.bench_function("tiles: wanted_tiles for one 1080p view", |b| {
        b.iter(|| {
            black_box(wanted_tiles(
                &cam,
                &layout.pages,
                vis.clone(),
                cam.scale(),
                |_| false,
            ))
        })
    });
}

fn bench_ink(c: &mut Criterion) {
    let mut store = Store::new(1);
    for i in 0..10_000 {
        store.push(wavy(0, 10.0 + (i % 700) as f32, 40));
    }
    // Build the grid once, then measure queries.
    let _ = store.hit_test(0, [100.0, 100.0], 5.0);
    let mut i = 0usize;
    c.bench_function("eraser: hit test among 10 000 strokes", |b| {
        b.iter(|| {
            i = (i + 37) % 500;
            black_box(store.hit_test(0, [60.0 + i as f32, 20.0 + (i * 3 % 600) as f32], 10.0))
        })
    });
    let s = wavy(0, 100.0, 400);
    c.bench_function("eraser: one stroke, 400 points", |b| {
        b.iter(|| black_box(eraser::hits(&s, [300.0, 103.0], 10.0)))
    });

    c.bench_function("history: add 1000 strokes then undo all", |b| {
        b.iter(|| {
            let mut st = Store::new(1);
            let mut h = History::default();
            for i in 0..1000 {
                h.add_stroke(&mut st, wavy(0, i as f32 * 0.5, 30));
            }
            while h.undo(&mut st).is_some() {}
        })
    });

    c.bench_function("smooth: finish a 600 point stroke", |b| {
        let raw: Vec<[f32; 2]> = (0..600)
            .map(|i| {
                [
                    i as f32 * 0.8,
                    (i as f32 * 0.07).sin() * 30.0 + ((i * 7919) % 13) as f32 * 0.1,
                ]
            })
            .collect();
        b.iter(|| {
            let mut p = raw.clone();
            let mut pr = None;
            mizu::ink::smooth::finish(&mut p, &mut pr, 0.1);
            black_box(p.len())
        })
    });
}

fn bench_input(c: &mut Criterion) {
    let maps = Keymaps::defaults();
    let seq = parse_seq("5jgg12Gzwj").unwrap();
    let mut eng = KeyEngine::default();
    let mut out = Vec::new();
    c.bench_function("keymap: feed 10 keys", |b| {
        b.iter(|| {
            out.clear();
            for k in &seq {
                eng.feed(&maps, Mode::Normal, *k, &mut out);
            }
            black_box(out.len())
        })
    });
}

fn bench_recolor(c: &mut Criterion) {
    let theme = DarkTheme::from_srgb8([255, 255, 255], [0, 0, 0]);
    let px: Vec<[f32; 3]> = (0..4096)
        .map(|i| {
            to_linear3([
                (i * 7 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 29 % 256) as u8,
            ])
        })
        .collect();
    c.bench_function("recolor (CPU reference): 4096 pixels", |b| {
        b.iter(|| {
            let mut acc = 0.0;
            for p in &px {
                acc += recolor(*p, &theme)[0];
            }
            black_box(acc)
        })
    });
}

fn bench_save(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let pages: Vec<PageSpec> = (0..20)
        .map(|i| PageSpec::new(595.0, 842.0, &format!("Page {i}")))
        .collect();
    let path = write_pdf(dir.path(), "save.pdf", &pages, &Extras::default());
    let strokes: Vec<Stroke> = (0..1000)
        .map(|i| wavy(i % 20, 50.0 + (i / 20) as f32 * 12.0, 40))
        .collect();
    let mut g = c.benchmark_group("save");
    g.sample_size(10);
    g.bench_function("1000 strokes into a 20 page PDF", |b| {
        b.iter(|| {
            annots::save_with_strokes(
                &path,
                &dir.path().join("out.pdf"),
                None,
                black_box(&strokes),
            )
            .unwrap()
        })
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_tiles,
    bench_view,
    bench_ink,
    bench_input,
    bench_recolor,
    bench_save
);
criterion_main!(benches);
