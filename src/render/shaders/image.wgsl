// Page tiles, previews and blank page backgrounds. One instance = one quad.

@group(1) @binding(0) var tex: texture_2d_array<f32>;

struct Inst {
    @location(0) rect: vec4<f32>,   // x, y, w, h in physical pixels
    @location(1) uv: vec4<f32>,     // u0, v0, u1, v1
    @location(2) layer: u32,        // 0xFFFFFFFF = plain white page
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    var o: VsOut;
    o.pos = to_ndc(inst.rect.xy + c * inst.rect.zw);
    o.uv = mix(inst.uv.xy, inst.uv.zw, c);
    o.layer = inst.layer;
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    var c = vec3<f32>(1.0);
    if (in.layer != 0xFFFFFFFFu) {
        c = textureSampleLevel(tex, samp, in.uv, i32(in.layer), 0.0).rgb;
    }
    return vec4<f32>(recolor(c), 1.0);
}
