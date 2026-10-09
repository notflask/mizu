//! `MIZU_DIAG=1` / `--diag`: describe what was actually drawn, so a bug
//! report can tell "the GPU drew nothing" apart from "it drew something
//! and the window system did not show it".

use super::Capture;

const RAMP: &[u8] = b" .:-=+*#%@";
const COLS: usize = 48;
const ROWS: usize = 12;

/// Plain-text summary of a captured frame: shares of non-black pixels, the
/// area they cover, and a coarse brightness map.
pub fn describe(cap: &Capture, bar_h: u32) -> String {
    let (w, h) = (cap.width as usize, cap.height as usize);
    if w == 0 || h == 0 || cap.rgba.len() < w * h * 4 {
        return "  (empty capture)".into();
    }
    let bar_top = h.saturating_sub(bar_h as usize);
    let mut non_black = 0usize;
    let mut bar_non_black = 0usize;
    let mut sum = [0u64; 3];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0usize, 0usize);
    let mut cells = [[0u64; COLS]; ROWS];
    let mut counts = [[0u64; COLS]; ROWS];

    for y in 0..h {
        let row = &cap.rgba[y * w * 4..(y + 1) * w * 4];
        let cy = y * ROWS / h;
        for x in 0..w {
            let p = &row[x * 4..x * 4 + 4];
            let luma = (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8;
            let cx = x * COLS / w;
            cells[cy][cx] += luma as u64;
            counts[cy][cx] += 1;
            for c in 0..3 {
                sum[c] += p[c] as u64;
            }
            if p[0] > 6 || p[1] > 6 || p[2] > 6 {
                non_black += 1;
                if y >= bar_top {
                    bar_non_black += 1;
                }
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    let total = (w * h) as f64;
    let n = total as u64;
    let mut out = format!(
        "  {w}x{h}, non-black {:.2} %, mean rgb ({}, {}, {}), status bar {} px high ({} lit px)\n",
        non_black as f64 / total * 100.0,
        sum[0] / n,
        sum[1] / n,
        sum[2] / n,
        bar_h,
        bar_non_black
    );
    if non_black == 0 {
        out.push_str("  the frame is completely black");
    } else {
        out.push_str(&format!("  lit area x {x0}..{x1}, y {y0}..{y1}\n"));
        for r in 0..ROWS {
            out.push_str("  |");
            for c in 0..COLS {
                let l = cells[r][c].checked_div(counts[r][c]).unwrap_or(0) as usize;
                out.push(RAMP[(l * (RAMP.len() - 1) + 127) / 255] as char);
            }
            out.push_str("|\n");
        }
        out.pop();
    }
    out
}

/// `MIZU_PRESENT_MODE=fifo|mailbox|immediate|auto`
pub fn present_mode_override() -> Option<wgpu::PresentMode> {
    let v = std::env::var("MIZU_PRESENT_MODE").ok()?;
    match v.to_ascii_lowercase().as_str() {
        "fifo" => Some(wgpu::PresentMode::Fifo),
        "mailbox" => Some(wgpu::PresentMode::Mailbox),
        "immediate" => Some(wgpu::PresentMode::Immediate),
        "auto" | "vsync" => Some(wgpu::PresentMode::AutoVsync),
        other => {
            log::warn!("MIZU_PRESENT_MODE={other}: expected fifo, mailbox, immediate or auto");
            None
        }
    }
}

/// `MIZU_SURFACE_FORMAT=bgra|rgba` picks the channel order of the swap chain
/// (always an sRGB format, the shaders rely on that).
pub fn format_override(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    let v = std::env::var("MIZU_SURFACE_FORMAT").ok()?;
    let want = match v.to_ascii_lowercase().as_str() {
        "bgra" | "bgra8" => wgpu::TextureFormat::Bgra8UnormSrgb,
        "rgba" | "rgba8" => wgpu::TextureFormat::Rgba8UnormSrgb,
        other => {
            log::warn!("MIZU_SURFACE_FORMAT={other}: expected bgra or rgba");
            return None;
        }
    };
    if formats.contains(&want) {
        Some(want)
    } else {
        log::warn!("MIZU_SURFACE_FORMAT: {want:?} is not offered by this surface ({formats:?})");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_is_reported() {
        let cap = Capture {
            width: 8,
            height: 8,
            rgba: vec![0; 8 * 8 * 4],
        };
        assert!(describe(&cap, 2).contains("completely black"));
    }

    #[test]
    fn lit_area_is_found() {
        let mut rgba = vec![0u8; 16 * 16 * 4];
        for y in 4..8 {
            for x in 2..6 {
                let i = (y * 16 + x) * 4;
                rgba[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let cap = Capture {
            width: 16,
            height: 16,
            rgba,
        };
        let text = describe(&cap, 2);
        assert!(text.contains("x 2..5, y 4..7"), "{text}");
        assert!(text.contains('@'));
    }
}
