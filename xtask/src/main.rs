//! Project tasks. `cargo xtask icons` renders the SVG sources in
//! `assets/icons/src` into every format the platforms need.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use resvg::tiny_skia::{self, Pixmap, PixmapPaint, Transform};
use resvg::usvg;

const APP_ID: &str = "io.github.notflask.Mizu";

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("icons") => icons(),
        _ => bail!("usage: cargo xtask icons"),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn render(svg: &str, size: u32) -> Result<Pixmap> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).context("parsing SVG")?;
    let mut pixmap = Pixmap::new(size, size).context("pixmap")?;
    let s = size as f32 / tree.size().width();
    resvg::render(&tree, Transform::from_scale(s, s), &mut pixmap.as_mut());
    Ok(pixmap)
}

/// Straight (non-premultiplied) RGBA bytes.
fn rgba(p: &Pixmap) -> Vec<u8> {
    let mut out = Vec::with_capacity(p.pixels().len() * 4);
    for px in p.pixels() {
        let c = px.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out
}

fn icons() -> Result<()> {
    let root = root();
    let src = root.join("assets/icons/src");
    let out = root.join("assets/icons/generated");
    let read = |n: &str| fs::read_to_string(src.join(n)).with_context(|| n.to_string());
    let (big, small, mac, symbolic) = (
        read("mizu.svg")?,
        read("mizu-small.svg")?,
        read("mizu-macos.svg")?,
        read("mizu-symbolic.svg")?,
    );
    // Small sizes use the simplified drawing.
    let pick = |size: u32| if size <= 32 { &small } else { &big };

    // --- Linux ------------------------------------------------------------
    let hicolor = out.join("linux/hicolor");
    fs::create_dir_all(hicolor.join("scalable/apps"))?;
    fs::create_dir_all(hicolor.join("symbolic/apps"))?;
    fs::write(hicolor.join(format!("scalable/apps/{APP_ID}.svg")), &big)?;
    fs::write(
        hicolor.join(format!("symbolic/apps/{APP_ID}-symbolic.svg")),
        &symbolic,
    )?;
    for size in [16u32, 24, 32, 48, 64, 128, 256, 512] {
        let dir = hicolor.join(format!("{size}x{size}/apps"));
        fs::create_dir_all(&dir)?;
        render(pick(size), size)?.save_png(dir.join(format!("{APP_ID}.png")))?;
    }

    // --- Windows ----------------------------------------------------------
    let mut ico = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16u32, 20, 24, 32, 40, 48, 64, 256] {
        let p = render(pick(size), size)?;
        let img = ico::IconImage::from_rgba_data(size, size, rgba(&p));
        ico.add_entry(ico::IconDirEntry::encode(&img)?);
    }
    fs::create_dir_all(out.join("windows"))?;
    ico.write(fs::File::create(out.join("windows/mizu.ico"))?)?;

    // --- macOS ------------------------------------------------------------
    let mut family = icns::IconFamily::new();
    let types = [
        (16, icns::IconType::RGBA32_16x16),
        (32, icns::IconType::RGBA32_16x16_2x),
        (64, icns::IconType::RGBA32_64x64),
        (128, icns::IconType::RGBA32_128x128),
        (256, icns::IconType::RGBA32_128x128_2x),
        (256, icns::IconType::RGBA32_256x256),
        (512, icns::IconType::RGBA32_256x256_2x),
        (512, icns::IconType::RGBA32_512x512),
        (1024, icns::IconType::RGBA32_512x512_2x),
    ];
    for (size, ty) in types {
        let p = render(&mac, size)?;
        let image = icns::Image::from_data(icns::PixelFormat::RGBA, size, size, rgba(&p))?;
        family.add_icon_with_type(&image, ty)?;
    }
    fs::create_dir_all(out.join("macos"))?;
    family.write(fs::File::create(out.join("macos/mizu.icns"))?)?;
    // A plain 1024 px PNG of the macOS artwork is handy for bundles and docs.
    render(&mac, 1024)?.save_png(out.join("macos/mizu-1024.png"))?;

    // --- window icon (X11 / Windows title bar) ------------------------------
    fs::write(out.join("window-icon-64.rgba"), rgba(&render(&big, 64)?))?;

    // --- preview sheet ------------------------------------------------------
    let mut sheet = Pixmap::new(1300, 560).context("sheet")?;
    for (row, bg) in [(0u32, [255u8, 255, 255]), (1, [28, 28, 30])] {
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(bg[0], bg[1], bg[2], 255);
        let rect =
            tiny_skia::Rect::from_xywh(0.0, row as f32 * 280.0, 1300.0, 280.0).context("rect")?;
        sheet.fill_rect(rect, &paint, Transform::identity(), None);
        let y = row as i32 * 280 + 12;
        let mut x = 20;
        for (svg, size) in [
            (&mac, 256u32),
            (&big, 256),
            (&big, 128),
            (&small, 64),
            (&small, 32),
            (&small, 16),
        ] {
            let p = render(svg, size)?;
            sheet.draw_pixmap(
                x,
                y + ((256 - size) / 2) as i32,
                p.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            x += size as i32 + 28;
        }
    }
    sheet.save_png(out.join("preview.png"))?;
    println!("icons written to {}", out.display());
    Ok(())
}
