// Flat rectangles and circles: status bar, search highlights, pen cursor.

struct Inst {
    @location(0) rect: vec4<f32>,    // x, y, w, h (px)
    @location(1) color: vec4<f32>,   // linear, straight alpha
    @location(2) params: vec4<f32>,  // x: shape (0 rect, 1 disc, 2 ring), y: ring thickness (px)
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,   // -1..1 inside the rect
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) params: vec4<f32>,
    @location(3) @interpolate(flat) size: vec2<f32>,
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
    o.local = c * 2.0 - vec2<f32>(1.0);
    o.color = inst.color;
    o.params = inst.params;
    o.size = inst.rect.zw;
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    var a = in.color.a;
    if (in.params.x > 0.5) {
        let r = in.size.x * 0.5;
        let dist = length(in.local * r);
        if (in.params.x < 1.5) {
            a = a * clamp(r - dist + 0.5, 0.0, 1.0);
        } else {
            let t = in.params.y * 0.5;
            let d = abs(dist - (r - t)) - t;
            a = a * clamp(0.5 - d, 0.0, 1.0);
        }
    }
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, a);
}
