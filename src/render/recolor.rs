//! CPU reference of the dark-mode recolouring that the shaders implement.
//!
//! The page's lightness is inverted in OKLab: white maps to the dark-mode
//! background, black to the foreground, everything in between on the line
//! between them. Hue and chroma of coloured content are kept; if a colour
//! would leave the sRGB gamut at its new lightness, chroma is reduced until
//! it fits (the hue stays put, unlike per-channel clamping).

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
    pub fg: [f32; 3],
    pub bg: [f32; 3],
}

impl DarkTheme {
    pub fn from_srgb8(fg: [u8; 3], bg: [u8; 3]) -> Self {
        DarkTheme {
            fg: to_oklab(to_linear3(fg)),
            bg: to_oklab(to_linear3(bg)),
        }
    }
}

/// Number of bisection steps for the chroma reduction. Keep in sync with
/// `recolor()` in `shaders/common.wgsl`.
pub const GAMUT_STEPS: usize = 8;

/// Recolour one linear-sRGB colour.
pub fn recolor(c: [f32; 3], theme: &DarkTheme) -> [f32; 3] {
    let lab = to_oklab(c);
    let t = lab[0].clamp(0.0, 1.0);
    let l = theme.fg[0] + (theme.bg[0] - theme.fg[0]) * t;
    let tint = [
        theme.fg[1] + (theme.bg[1] - theme.fg[1]) * t,
        theme.fg[2] + (theme.bg[2] - theme.fg[2]) * t,
    ];
    let full = from_oklab([l, tint[0] + lab[1], tint[1] + lab[2]]);
    if in_gamut(full) {
        return full.map(|v| v.clamp(0.0, 1.0));
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..GAMUT_STEPS {
        let mid = 0.5 * (lo + hi);
        let cand = from_oklab([l, tint[0] + lab[1] * mid, tint[1] + lab[2] * mid]);
        if in_gamut(cand) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    from_oklab([l, tint[0] + lab[1] * lo, tint[1] + lab[2] * lo]).map(|v| v.clamp(0.0, 1.0))
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
        for v in (0..=255u8).step_by(15) {
            let out = recolor_srgb8([v, v, v], &t);
            assert!(out[0] == out[1] && out[1] == out[2], "grey {v} -> {out:?}");
            assert!((out[0] as i32) < prev, "not monotonic at {v}");
            prev = out[0] as i32;
        }
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
    fn dark_ink_becomes_light() {
        // A typical pen colour (#1a1a1a) must end up light on the dark page.
        let t = black_white();
        let out = recolor_srgb8([0x1a, 0x1a, 0x1a], &t);
        assert!(out[0] > 200, "{out:?}");
    }
}
