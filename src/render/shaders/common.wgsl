// Shared by all pipelines: globals and the dark-mode recolouring.
// `recolor()` mirrors `render/recolor.rs`; keep the two in sync.

struct Globals {
    viewport: vec2<f32>,
    dark: u32,
    _pad: u32,
    // OKLab of the dark-mode foreground / background (xyz, w unused).
    fg: vec4<f32>,
    bg: vec4<f32>,
    // The same colours in linear sRGB.
    fg_lin: vec4<f32>,
    bg_lin: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var samp: sampler;

fn to_ndc(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x / g.viewport.x * 2.0 - 1.0, 1.0 - p.y / g.viewport.y * 2.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn cbrt3(v: vec3<f32>) -> vec3<f32> {
    return sign(v) * pow(abs(v), vec3<f32>(1.0 / 3.0));
}

fn to_oklab(c: vec3<f32>) -> vec3<f32> {
    let l = 0.4122214708 * c.r + 0.5363325363 * c.g + 0.0514459929 * c.b;
    let m = 0.2119034982 * c.r + 0.6806995451 * c.g + 0.1073969566 * c.b;
    let s = 0.0883024619 * c.r + 0.2817188376 * c.g + 0.6299787005 * c.b;
    let q = cbrt3(vec3<f32>(l, m, s));
    return vec3<f32>(
        0.2104542553 * q.x + 0.7936177850 * q.y - 0.0040720468 * q.z,
        1.9779984951 * q.x - 2.4285922050 * q.y + 0.4505937099 * q.z,
        0.0259040371 * q.x + 0.7827717662 * q.y - 0.8086757660 * q.z,
    );
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.3963377774 * lab.y + 0.2158037573 * lab.z;
    let m_ = lab.x - 0.1055613458 * lab.y - 0.0638541728 * lab.z;
    let s_ = lab.x - 0.0894841775 * lab.y - 1.2914855480 * lab.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    return vec3<f32>(
        4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    );
}

fn in_gamut(c: vec3<f32>) -> bool {
    let e = 0.0001;
    return all(c >= vec3<f32>(-e)) && all(c <= vec3<f32>(1.0 + e));
}

fn recolor(c: vec3<f32>) -> vec3<f32> {
    if (g.dark == 0u) {
        return c;
    }
    let lab = to_oklab(c);
    let y = clamp(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    // Neutral path: mix in linear light (exact anti-aliasing for text).
    let base_lin = mix(g.fg_lin.xyz, g.bg_lin.xyz, y);
    let base = to_oklab(base_lin);
    // Coloured path: invert OKLab lightness, keep hue and chroma.
    let t = clamp(lab.x, 0.0, 1.0);
    // Inverted lightness with lifted mid-tones (COLOR_LIFT = 0.55 in recolor.rs).
    let l_inv = mix(g.fg.x, g.bg.x, 1.0 - pow(1.0 - t, 0.55));
    let chroma = length(lab.yz);
    let w = smoothstep(0.02, 0.10, chroma);
    let l = mix(base.x, l_inv, w);

    let full = from_oklab(vec3<f32>(l, base.yz + lab.yz));
    if (in_gamut(full)) {
        return clamp(full, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    var lo = 0.0;
    var hi = 1.0;
    for (var i = 0; i < 8; i++) {
        let mid = 0.5 * (lo + hi);
        let cand = from_oklab(vec3<f32>(l, base.yz + lab.yz * mid));
        if (in_gamut(cand)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    return clamp(from_oklab(vec3<f32>(l, base.yz + lab.yz * lo)), vec3<f32>(0.0), vec3<f32>(1.0));
}
