//! CPU reference of the dark-mode recolouring that the shaders implement.
//!
//! Neutral tones (text, backgrounds, anti-aliased glyph edges) are mapped in
//! *linear light*: white becomes the dark-mode background, black the
//! foreground, and a grey in between is the matching mix. That makes the
//! anti-aliasing of black-on-white text come out exactly right as
//! light-on-dark text (a pixel that was 30 % ink is 30 % foreground).
//!
//! Coloured pixels keep hue and chroma; their lightness is inverted in OKLab
//! so reds stay vivid instead of turning pale. The two paths are blended by
//! chroma. If a colour would leave the sRGB gamut at its new lightness, chroma
//! is reduced until it fits (the hue stays put, unlike per-channel clamping).

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn to_linear3(rgb8: [u8; 3]) -> [f32; 3] {
    [
        srgb_to_linear(rgb8[0] as f32 / 255.0),
        srgb_to_linear(rgb8[1] as f32 / 255.0),
        srgb_to_linear(rgb8[2] as f32 / 255.0),
    ]
}

pub fn to_oklab(c: [f32; 3]) -> [f32; 3] {
    let l = 0.412_221_46 * c[0] + 0.536_332_55 * c[1] + 0.051_445_995 * c[2];
    let m = 0.211_903_5 * c[0] + 0.680_699_5 * c[1] + 0.107_396_96 * c[2];
    let s = 0.088_302_46 * c[0] + 0.281_718_85 * c[1] + 0.629_978_7 * c[2];
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn from_oklab(lab: [f32; 3]) -> [f32; 3] {
    let l = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m = lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2];
    let s = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

fn in_gamut(c: [f32; 3]) -> bool {
    const E: f32 = 1e-4;
    c.iter().all(|&v| (-E..=1.0 + E).contains(&v))
}

/// Parameters of the dark theme in OKLab.
#[derive(Clone, Copy, Debug)]
pub struct DarkTheme {
    /// OKLab of the foreground / background.
    pub fg: [f32; 3],
    pub bg: [f32; 3],
    /// The same colours in linear sRGB.
    pub fg_lin: [f32; 3],
    pub bg_lin: [f32; 3],
}

impl DarkTheme {
    pub fn from_srgb8(fg: [u8; 3], bg: [u8; 3]) -> Self {
        let (fg_lin, bg_lin) = (to_linear3(fg), to_linear3(bg));
        DarkTheme {
            fg: to_oklab(fg_lin),
            bg: to_oklab(bg_lin),
            fg_lin,
            bg_lin,
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Exponent that lifts the inverted lightness of coloured pixels.
pub const COLOR_LIFT: f32 = 0.55;

/// Chroma range over which the neutral path hands over to the coloured one.
pub const CHROMA_LO: f32 = 0.02;
pub const CHROMA_HI: f32 = 0.10;

/// Number of bisection steps for the chroma reduction. Keep in sync with
/// `recolor()` in `shaders/common.wgsl`.
pub const GAMUT_STEPS: usize = 8;

/// Recolour one linear-sRGB colour.
pub fn recolor(c: [f32; 3], theme: &DarkTheme) -> [f32; 3] {
    let lab = to_oklab(c);
    let y = (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0);
    // Neutral path: mix in linear light.
    let base_lin = [
        theme.fg_lin[0] + (theme.bg_lin[0] - theme.fg_lin[0]) * y,
        theme.fg_lin[1] + (theme.bg_lin[1] - theme.fg_lin[1]) * y,
        theme.fg_lin[2] + (theme.bg_lin[2] - theme.fg_lin[2]) * y,
    ];
    let base = to_oklab(base_lin);
    // Coloured path: invert OKLab lightness.
    let t = lab[0].clamp(0.0, 1.0);
    // Inverted lightness, with the mid-tones lifted so that saturated text
    // and lines stay readable while pale fills still go dark.
    let s = 1.0 - (1.0 - t).powf(COLOR_LIFT);
    let l_inv = theme.fg[0] + (theme.bg[0] - theme.fg[0]) * s;
    let chroma = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let w = smoothstep(CHROMA_LO, CHROMA_HI, chroma);
    let l = base[0] + (l_inv - base[0]) * w;

    let full = from_oklab([l, base[1] + lab[1], base[2] + lab[2]]);
    if in_gamut(full) {
        return full.map(|v| v.clamp(0.0, 1.0));
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..GAMUT_STEPS {
        let mid = 0.5 * (lo + hi);
        let cand = from_oklab([l, base[1] + lab[1] * mid, base[2] + lab[2] * mid]);
        if in_gamut(cand) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    from_oklab([l, base[1] + lab[1] * lo, base[2] + lab[2] * lo]).map(|v| v.clamp(0.0, 1.0))
}

/// Convenience for tests and tools: sRGB8 in, sRGB8 out.
pub fn recolor_srgb8(c: [u8; 3], theme: &DarkTheme) -> [u8; 3] {
    let out = recolor(to_linear3(c), theme);
    [
        (linear_to_srgb(out[0]) * 255.0).round() as u8,
        (linear_to_srgb(out[1]) * 255.0).round() as u8,
        (linear_to_srgb(out[2]) * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black_white() -> DarkTheme {
        DarkTheme::from_srgb8([255, 255, 255], [0, 0, 0])
    }

    fn hue(c: [u8; 3]) -> f32 {
        let lab = to_oklab(to_linear3(c));
        lab[2].atan2(lab[1]).to_degrees()
    }

    fn chroma(c: [u8; 3]) -> f32 {
        let lab = to_oklab(to_linear3(c));
        (lab[1] * lab[1] + lab[2] * lab[2]).sqrt()
    }

    #[test]
    fn oklab_roundtrip() {
        for c in [
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.2, 0.5, 0.8],
            [0.9, 0.1, 0.3],
        ] {
            let back = from_oklab(to_oklab(c));
            for k in 0..3 {
                assert!((c[k] - back[k]).abs() < 1e-3, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn white_becomes_background_black_becomes_foreground() {
        let t = black_white();
        assert_eq!(recolor_srgb8([255, 255, 255], &t), [0, 0, 0]);
        assert_eq!(recolor_srgb8([0, 0, 0], &t), [255, 255, 255]);
    }

    #[test]
    fn works_for_custom_colours_too() {
        let t = DarkTheme::from_srgb8([0xee, 0xee, 0xee], [0x10, 0x10, 0x18]);
        let w = recolor_srgb8([255, 255, 255], &t);
        let k = recolor_srgb8([0, 0, 0], &t);
        for (got, want) in w.iter().zip([0x10u8, 0x10, 0x18]) {
            assert!((*got as i32 - want as i32).abs() <= 1, "{w:?}");
        }
        for (got, want) in k.iter().zip([0xeeu8, 0xee, 0xee]) {
            assert!((*got as i32 - want as i32).abs() <= 1, "{k:?}");
        }
    }

    #[test]
    fn greys_stay_grey_and_invert_monotonically() {
        let t = black_white();
        let mut prev = 256i32;
        for v in (0..=255u8).step_by(5) {
            let out = recolor_srgb8([v, v, v], &t);
            assert!(out[0] == out[1] && out[1] == out[2], "grey {v} -> {out:?}");
            // Non-increasing everywhere; dark inputs may saturate at white
            // after rounding, so strictness is only required in the middle.
            assert!((out[0] as i32) <= prev, "not monotonic at {v}");
            if (60..200).contains(&v) {
                assert!((out[0] as i32) < prev, "flat in the mid range at {v}");
            }
            prev = out[0] as i32;
        }
        assert_eq!(recolor_srgb8([0, 0, 0], &t), [255, 255, 255]);
        assert_eq!(prev, 0);
    }

    #[test]
    fn saturated_colours_keep_their_hue() {
        let t = black_white();
        let samples: [[u8; 3]; 6] = [
            [255, 0, 0],
            [0, 160, 0],
            [0, 80, 255],
            [255, 140, 0],
            [160, 40, 200],
            [200, 30, 30],
        ];
        for c in samples {
            let out = recolor_srgb8(c, &t);
            let dh = (hue(c) - hue(out)).abs();
            let dh = dh.min(360.0 - dh);
            assert!(dh < 15.0, "{c:?} -> {out:?}: hue moved by {dh:.1}°");
            assert!(chroma(out) > 0.02, "{c:?} lost its colour: {out:?}");
        }
    }

    #[test]
    fn output_is_always_in_gamut() {
        let t = black_white();
        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    let out = recolor(to_linear3([r as u8, g as u8, b as u8]), &t);
                    assert!(
                        out.iter().all(|v| (0.0..=1.0).contains(v)),
                        "{r},{g},{b} -> {out:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn saturated_text_colours_stay_readable_and_pale_fills_go_dark() {
        let t = black_white();
        let lightness = |c: [u8; 3]| to_oklab(to_linear3(recolor_srgb8(c, &t)))[0];
        // Pure red / green / blue on a black page: clearly visible.
        assert!(lightness([255, 0, 0]) > 0.5, "red");
        assert!(lightness([0, 130, 40]) > 0.5, "green");
        assert!(lightness([40, 80, 220]) > 0.5, "blue");
        // A pale yellow highlight box must not become a bright block.
        assert!(lightness([255, 237, 178]) < 0.3, "pale yellow");
    }

    #[test]
    fn dark_ink_becomes_light() {
        // A typical pen colour (#1a1a1a) must end up light on the dark page.
        let t = black_white();
        let out = recolor_srgb8([0x1a, 0x1a, 0x1a], &t);
        assert!(out[0] > 235, "{out:?}");
    }

    #[test]
    fn antialiased_edges_blend_like_real_light() {
        // A pixel that is `c` black ink over white paper has linear value 1-c.
        // On the dark page the same pixel must be `c` foreground over background.
        let t = black_white();
        for i in 1..10 {
            let c = i as f32 / 10.0;
            let out = recolor([1.0 - c; 3], &t);
            for ch in out {
                assert!((ch - c).abs() < 0.01, "coverage {c}: got {out:?}");
            }
        }
        // And with a non-black background the mix is between bg and fg.
        let t = DarkTheme::from_srgb8([255, 255, 255], [0x20, 0x20, 0x20]);
        let half = recolor([0.5; 3], &t);
        let expect = (to_linear3([255, 255, 255])[0] + to_linear3([0x20, 0x20, 0x20])[0]) * 0.5;
        assert!((half[0] - expect).abs() < 0.01, "{half:?} vs {expect}");
    }

    #[test]
    fn near_neutral_pixels_do_not_flicker_between_paths() {
        // Tiny chroma differences must not cause jumps in the output.
        let t = black_white();
        let a = recolor([0.30, 0.30, 0.30], &t);
        let b = recolor([0.31, 0.30, 0.29], &t);
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 0.03, "{a:?} vs {b:?}");
        }
    }
}
