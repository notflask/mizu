// Ink. Every segment of a stroke is one instance: a tapered capsule that is
// shaded from its signed distance field, so round caps/joins and edge
// anti-aliasing are exact at any zoom without tessellation.

struct PageU {
    origin: vec2<f32>,  // screen position of the page's top-left corner (px)
    scale: f32,         // physical pixels per point
    _pad: f32,
};
@group(1) @binding(0) var<uniform> page: PageU;

struct Inst {
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) r: vec2<f32>,      // radii at a and b (points)
    @location(3) color: vec4<f32>,  // sRGB, from Unorm8x4
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) p: vec2<f32>,                       // point in page space
    @location(1) @interpolate(flat) a: vec2<f32>,
    @location(2) @interpolate(flat) b: vec2<f32>,
    @location(3) @interpolate(flat) r: vec2<f32>,
    @location(4) @interpolate(flat) color: vec3<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let d = inst.b - inst.a;
    let len = length(d);
    var dir = vec2<f32>(1.0, 0.0);
    if (len > 1e-6) {
        dir = d / len;
    }
    let n = vec2<f32>(-dir.y, dir.x);
    let m = 2.0 / page.scale;               // anti-aliasing margin
    let rmax = max(inst.r.x, inst.r.y) + m;
    let along = mix(-(inst.r.x + m), len + inst.r.y + m, c.x);
    let across = mix(-rmax, rmax, c.y);
    let p = inst.a + dir * along + n * across;

    var o: VsOut;
    o.pos = to_ndc(page.origin + p * page.scale);
    o.p = p;
    o.a = inst.a;
    o.b = inst.b;
    o.r = inst.r;
    o.color = srgb_to_linear(inst.color.rgb);
    return o;
}

// Signed distance to a capsule whose radius changes linearly from ra to rb.
fn sd_capsule(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, ra: f32, rb: f32) -> f32 {
    let ba = b - a;
    let h = length(ba);
    // One circle swallows the other (or a dot): just a circle.
    if (h <= abs(ra - rb) + 1e-6) {
        if (ra >= rb) {
            return length(p - a) - ra;
        }
        return length(p - b) - rb;
    }
    let dir = ba / h;
    let rel = p - a;
    var q = vec2<f32>(abs(dot(rel, vec2<f32>(-dir.y, dir.x))), dot(rel, dir));
    let bb = (ra - rb) / h;
    let aa = sqrt(1.0 - bb * bb);
    let k = dot(q, vec2<f32>(-bb, aa));
    if (k < 0.0) {
        return length(q) - ra;
    }
    if (k > aa * h) {
        return length(q - vec2<f32>(0.0, h)) - rb;
    }
    return dot(q, vec2<f32>(aa, bb)) - ra;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let sd = sd_capsule(in.p, in.a, in.b, in.r.x, in.r.y);
    let alpha = clamp(0.5 - sd * page.scale, 0.0, 1.0);
    if (alpha <= 0.0) {
        discard;
    }
    return vec4<f32>(recolor(in.color), alpha);
}
