//! The renderer: one render pass per frame, driven by pre-built pipelines
//! and persistent buffers. Pages are drawn from cached tiles, ink from
//! per-page instance buffers, UI from glyphon.

pub mod atlas;
pub mod gpu;
pub mod overlay;
pub mod recolor;
pub mod tiles;
pub mod ui;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use bytemuck::{Pod, Zeroable};
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::doc::worker::{ThumbPixels, TileKey, TilePixels, THUMB, TILE};
use crate::ink::stroke::pressure_width;
use crate::ink::{Store, Stroke};
use crate::view::{Camera, Layout};

use atlas::SlotArrays;
use gpu::{Acquired, Gpu};
use overlay::OverlayInst;
use tiles::{ThumbEntry, TileEntry, TileIndex};
pub use ui::{ListOverlay, UiState};

const SOLID: u32 = u32::MAX;
const THUMB_BUDGET_MB: u32 = 48;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageInst {
    rect: [f32; 4],
    uv: [f32; 4],
    layer: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct StrokeInst {
    a: [f32; 2],
    b: [f32; 2],
    r: [f32; 2],
    color: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    dark: u32,
    _pad: u32,
    fg: [f32; 4],
    bg: [f32; 4],
    fg_lin: [f32; 4],
    bg_lin: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PageU {
    origin: [f32; 2],
    scale: f32,
    _pad: f32,
}

/// A buffer that grows (never shrinks) to fit what is written to it.
struct GrowBuf {
    buf: wgpu::Buffer,
    cap: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowBuf {
    fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        cap: u64,
    ) -> Self {
        GrowBuf {
            buf: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: cap,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            cap,
            usage,
            label,
        }
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        let need = data.len() as u64;
        if need == 0 {
            return;
        }
        if need > self.cap {
            let cap = need.next_power_of_two().max(1024);
            *self = GrowBuf::new(device, self.label, self.usage, cap);
        }
        queue.write_buffer(&self.buf, 0, data);
    }
}

/// Ink of one page on the GPU.
struct PageInk {
    buf: GrowBuf,
    count: u32,
    generation: u64,
}

/// What the pen is drawing right now (not yet part of the document).
pub struct LiveStroke<'a> {
    pub page: usize,
    pub points: &'a [[f32; 2]],
    pub pressure: Option<&'a [f32]>,
    pub width: f32,
    pub color: [u8; 3],
}

pub struct Highlight {
    pub page: usize,
    /// `[x0, y0, x1, y1]` in page space.
    pub rect: [f32; 4],
    pub current: bool,
}

pub struct PenCursor {
    /// Screen position in physical pixels.
    pub pos: [f32; 2],
    /// Radius in physical pixels.
    pub radius: f32,
    pub color: [u8; 3],
    pub eraser: bool,
}

pub struct Theme {
    pub dark: bool,
    pub dark_bg: [u8; 3],
    pub dark_fg: [u8; 3],
    pub separator: Option<[u8; 3]>,
}

pub struct FrameInput<'a> {
    pub camera: &'a Camera,
    pub layout: &'a Layout,
    /// Scale (physical px per point) the visible tiles were rendered at.
    pub tile_scale: f32,
    pub theme: &'a Theme,
    pub ink: &'a Store,
    pub live: Option<LiveStroke<'a>>,
    pub highlights: &'a [Highlight],
    pub cursor: Option<PenCursor>,
    pub ui: &'a UiState,
    pub current_page: usize,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub image_instances: u32,
    pub stroke_instances: u32,
    pub tiles_cached: u32,
    pub tile_slots: u32,
}

type KindMaker = fn(u16) -> BatchKind;

#[derive(Clone, Copy)]
enum BatchKind {
    Solid,
    Thumb(u16),
    Tile(u16),
}

struct Batch {
    kind: BatchKind,
    first: u32,
    count: u32,
}

pub struct Renderer {
    gpu: Gpu,
    // pipelines
    image_pipe: wgpu::RenderPipeline,
    stroke_pipe: wgpu::RenderPipeline,
    overlay_pipe: wgpu::RenderPipeline,
    /// Multiplies with the page, so dark text stays dark under a highlight.
    multiply_pipe: wgpu::RenderPipeline,
    // bind groups
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    tex_bgl: wgpu::BindGroupLayout,
    page_u_buf: GrowBuf,
    page_u_bg: wgpu::BindGroup,
    page_u_bgl: wgpu::BindGroupLayout,
    page_u_stride: u64,
    // dynamic data
    image_buf: GrowBuf,
    overlay_buf: GrowBuf,
    live_buf: GrowBuf,
    page_ink: HashMap<usize, PageInk>,
    // cache
    tile_arrays: SlotArrays,
    thumb_arrays: SlotArrays,
    index: TileIndex,
    // ui
    ui: ui::Ui,
    // scratch (reused every frame)
    images: Vec<ImageInst>,
    batches: Vec<Batch>,
    rects_page: Vec<OverlayInst>,
    rects_hl: Vec<OverlayInst>,
    rects_ui: Vec<OverlayInst>,
    page_us: Vec<PageU>,
    stroke_scratch: Vec<StrokeInst>,
    buckets: [Vec<Vec<ImageInst>>; 3],
    capture_requested: bool,
    captured: Option<Capture>,
}

/// A screenshot of the last frame (RGBA8, sRGB).
pub struct Capture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const GLOBALS_SIZE: u64 = std::mem::size_of::<Globals>() as u64;

fn module(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
    let src = format!("{}\n{}", include_str!("shaders/common.wgsl"), body);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(src.into()),
    })
}

impl Renderer {
    pub fn new(
        window: Arc<Window>,
        event_loop: &ActiveEventLoop,
        tile_cache_mb: u32,
        capture: bool,
    ) -> Result<Renderer> {
        let gpu = Gpu::new(window, event_loop, capture)?;
        let device = &gpu.device;
        let format = gpu.format();

        let globals_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let tex_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tiles"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let page_u_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("page-uniform"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<PageU>() as u64),
                },
                count: None,
            }],
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: GLOBALS_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let page_u_stride = (std::mem::size_of::<PageU>() as u64).div_ceil(align) * align;
        let page_u_buf = GrowBuf::new(
            device,
            "page-uniforms",
            wgpu::BufferUsages::UNIFORM,
            page_u_stride * 32,
        );
        let page_u_bg = Self::make_page_u_bg(device, &page_u_bgl, &page_u_buf);

        let img_mod = module(device, "image", include_str!("shaders/image.wgsl"));
        let stroke_mod = module(device, "stroke", include_str!("shaders/stroke.wgsl"));
        let overlay_mod = module(device, "overlay", include_str!("shaders/overlay.wgsl"));

        let layout_for = |bgls: &[Option<&wgpu::BindGroupLayout>]| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: bgls,
                immediate_size: 0,
            })
        };
        let blend_target = |blend: Option<wgpu::BlendState>| {
            [Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })]
        };
        let make = |label: &str,
                    module: &wgpu::ShaderModule,
                    layout: &wgpu::PipelineLayout,
                    attrs: &[wgpu::VertexAttribute],
                    stride: u64,
                    blend: Option<wgpu::BlendState>| {
            let buffers = [Some(wgpu::VertexBufferLayout {
                array_stride: stride,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: attrs,
            })];
            let targets = blend_target(blend);
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &buffers,
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };

        let image_pipe = make(
            "image",
            &img_mod,
            &layout_for(&[Some(&globals_bgl), Some(&tex_bgl)]),
            &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Uint32],
            std::mem::size_of::<ImageInst>() as u64,
            None,
        );
        let stroke_pipe = make(
            "stroke",
            &stroke_mod,
            &layout_for(&[Some(&globals_bgl), Some(&page_u_bgl)]),
            &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Unorm8x4],
            std::mem::size_of::<StrokeInst>() as u64,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );
        let overlay_pipe = make(
            "overlay",
            &overlay_mod,
            &layout_for(&[Some(&globals_bgl)]),
            &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
            std::mem::size_of::<OverlayInst>() as u64,
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let multiply_pipe = make(
            "overlay-multiply",
            &overlay_mod,
            &layout_for(&[Some(&globals_bgl)]),
            &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
            std::mem::size_of::<OverlayInst>() as u64,
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::REPLACE,
            }),
        );
        let tile_arrays = SlotArrays::new(TILE, tile_cache_mb, device, "tiles");
        let thumb_arrays = SlotArrays::new(THUMB, THUMB_BUDGET_MB, device, "previews");
        let ui = ui::Ui::new(device, &gpu.queue, format);
        let image_buf = GrowBuf::new(device, "images", wgpu::BufferUsages::VERTEX, 64 * 1024);
        let overlay_buf = GrowBuf::new(device, "overlays", wgpu::BufferUsages::VERTEX, 16 * 1024);
        let live_buf = GrowBuf::new(device, "live-stroke", wgpu::BufferUsages::VERTEX, 16 * 1024);

        Ok(Renderer {
            gpu,
            image_pipe,
            stroke_pipe,
            overlay_pipe,
            multiply_pipe,
            globals_buf,
            globals_bg,
            tex_bgl,
            page_u_buf,
            page_u_bg,
            page_u_bgl,
            page_u_stride,
            image_buf,
            overlay_buf,
            live_buf,
            page_ink: HashMap::new(),
            tile_arrays,
            thumb_arrays,
            index: TileIndex::default(),
            ui,
            images: Vec::new(),
            batches: Vec::new(),
            rects_page: Vec::new(),
            rects_hl: Vec::new(),
            rects_ui: Vec::new(),
            page_us: Vec::new(),
            stroke_scratch: Vec::new(),
            buckets: [Vec::new(), Vec::new(), Vec::new()],
            capture_requested: false,
            captured: None,
        })
    }

    fn make_page_u_bg(
        device: &wgpu::Device,
        bgl: &wgpu::BindGroupLayout,
        buf: &GrowBuf,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("page-uniform"),
            layout: bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buf.buf,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<PageU>() as u64),
                }),
            }],
        })
    }

    /// Ask for a screenshot of the next frame (needs the `capture` flag at startup).
    pub fn request_capture(&mut self) {
        self.capture_requested = true;
    }

    pub fn take_capture(&mut self) -> Option<Capture> {
        self.captured.take()
    }

    pub fn window(&self) -> &Arc<Window> {
        &self.gpu.window
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.gpu.resize(w, h);
    }

    pub fn surface_size(&self) -> [u32; 2] {
        self.gpu.size()
    }

    pub fn bar_height(&self) -> f32 {
        self.ui.bar_height
    }

    pub fn has_tile(&self, key: &TileKey) -> bool {
        self.index.tiles.contains_key(key)
    }

    pub fn has_thumb(&self, page: u32) -> bool {
        self.index.thumbs.contains_key(&page)
    }

    pub fn cache_usage(&self) -> (usize, usize) {
        (self.tile_arrays.in_use(), self.tile_arrays.capacity())
    }

    /// Forget every cached tile and preview (after a reload).
    pub fn clear_cache(&mut self) {
        let keys: Vec<TileKey> = self.index.tiles.keys().copied().collect();
        for k in keys {
            if let Some(e) = self.index.remove(&k) {
                self.tile_arrays.release(e.slot);
            }
        }
        let pages: Vec<u32> = self.index.thumbs.keys().copied().collect();
        for p in pages {
            if let Some(e) = self.index.thumbs.remove(&p) {
                self.thumb_arrays.release(e.slot);
            }
        }
        self.page_ink.clear();
    }

    /// Upload a rendered tile. Returns false if there was nowhere to put it.
    pub fn insert_tile(&mut self, t: &TilePixels) -> bool {
        if self.index.tiles.contains_key(&t.key) {
            return true;
        }
        let slot = loop {
            if let Some(s) = self.tile_arrays.alloc(&self.gpu.device, &self.tex_bgl) {
                break s;
            }
            match self.index.lru_tile() {
                Some(k) => {
                    if let Some(e) = self.index.remove(&k) {
                        self.tile_arrays.release(e.slot);
                    }
                }
                None => return false,
            }
        };
        self.tile_arrays
            .upload(&self.gpu.queue, slot, t.w as u32, t.h as u32, &t.data);
        self.index.insert(
            t.key,
            TileEntry {
                slot,
                w: t.w,
                h: t.h,
                last_used: self.index.frame,
            },
        );
        true
    }

    pub fn insert_thumb(&mut self, t: &ThumbPixels, current_page: usize) -> bool {
        if self.index.thumbs.contains_key(&t.page) {
            return true;
        }
        let slot = loop {
            if let Some(s) = self.thumb_arrays.alloc(&self.gpu.device, &self.tex_bgl) {
                break s;
            }
            match self.index.farthest_thumb(current_page) {
                Some(p) => {
                    if let Some(e) = self.index.thumbs.remove(&p) {
                        self.thumb_arrays.release(e.slot);
                    }
                }
                None => return false,
            }
        };
        self.thumb_arrays
            .upload(&self.gpu.queue, slot, t.w as u32, t.h as u32, &t.data);
        self.index.thumbs.insert(
            t.page,
            ThumbEntry {
                slot,
                w: t.w,
                h: t.h,
                last_used: self.index.frame,
            },
        );
        true
    }

    fn stroke_instances(s: &Stroke, out: &mut Vec<StrokeInst>) {
        let c = [s.color[0], s.color[1], s.color[2], 255];
        let r = |i: usize| match &s.pressure {
            Some(p) => pressure_width(s.width, p.get(i).copied().unwrap_or(1.0)) * 0.5,
            None => s.width * 0.5,
        };
        match s.points.len() {
            0 => {}
            1 => out.push(StrokeInst {
                a: s.points[0],
                b: s.points[0],
                r: [r(0), r(0)],
                color: c,
            }),
            n => {
                for i in 0..n - 1 {
                    out.push(StrokeInst {
                        a: s.points[i],
                        b: s.points[i + 1],
                        r: [r(i), r(i + 1)],
                        color: c,
                    });
                }
            }
        }
    }

    /// Bring the GPU copy of a page's ink up to date.
    fn sync_page_ink(&mut self, page: usize, ink: &Store) {
        let generation = ink.generation(page);
        if let Some(p) = self.page_ink.get(&page) {
            if p.generation == generation {
                return;
            }
        }
        self.stroke_scratch.clear();
        for s in ink.strokes(page) {
            Self::stroke_instances(s, &mut self.stroke_scratch);
        }
        let count = self.stroke_scratch.len() as u32;
        let device = &self.gpu.device;
        let entry = self.page_ink.entry(page).or_insert_with(|| PageInk {
            buf: GrowBuf::new(device, "page-ink", wgpu::BufferUsages::VERTEX, 4096),
            count: 0,
            generation: u64::MAX,
        });
        entry.buf.write(
            device,
            &self.gpu.queue,
            bytemuck::cast_slice(&self.stroke_scratch),
        );
        entry.count = count;
        entry.generation = generation;
    }

    fn push_image(&mut self, bucket: usize, array: u16, inst: ImageInst) {
        let b = &mut self.buckets[bucket];
        if b.len() <= array as usize {
            b.resize_with(array as usize + 1, Vec::new);
        }
        b[array as usize].push(inst);
    }

    /// Build the instance lists for pages, previews and tiles.
    fn build_images(&mut self, input: &FrameInput) {
        let cam = input.camera;
        let layout = input.layout;
        let ts = input.tile_scale;
        let cs = cam.scale();
        let ratio = cs / ts;
        for b in &mut self.buckets {
            for v in b.iter_mut() {
                v.clear();
            }
        }
        self.images.clear();
        self.batches.clear();
        let mut solids: Vec<ImageInst> = Vec::new();
        let frame = self.index.frame;
        let area_h = (cam.viewport[1]).max(1.0);
        let [x0, y0, x1, y1] = cam.visible_doc_rect();
        let _ = (x0, x1);
        let vis = layout.visible(y0, y1);
        let _ = area_h;

        for pi in vis {
            let g = &layout.pages[pi];
            let origin = tiles::page_origin(cam, g);
            let ppx = tiles::page_px(g, ts);
            solids.push(ImageInst {
                rect: [
                    origin[0],
                    origin[1],
                    ppx.0 as f32 * ratio,
                    ppx.1 as f32 * ratio,
                ],
                uv: [0.0; 4],
                layer: SOLID,
                _pad: [0; 3],
            });

            // Low-resolution preview under everything else.
            if let Some(t) = self.index.thumbs.get_mut(&(pi as u32)) {
                t.last_used = frame;
                let s = tiles::thumb_scale(g);
                let inst = ImageInst {
                    rect: [
                        origin[0],
                        origin[1],
                        t.w as f32 / s * cs,
                        t.h as f32 / s * cs,
                    ],
                    uv: [
                        0.0,
                        0.0,
                        t.w as f32 / THUMB as f32,
                        t.h as f32 / THUMB as f32,
                    ],
                    layer: t.slot.layer as u32,
                    _pad: [0; 3],
                };
                let arr = t.slot.array;
                self.push_image(0, arr, inst);
            }

            // Tiles at the current scale; remember if any is missing.
            let mut missing = false;
            if let Some(r) = tiles::tile_range(origin, ratio, ppx, cam.viewport, 0.0) {
                for ty in r.ty0..=r.ty1 {
                    for tx in r.tx0..=r.tx1 {
                        let key = TileKey {
                            page: pi as u32,
                            scale: ts.to_bits(),
                            tx: tx as u16,
                            ty: ty as u16,
                        };
                        match self.index.tiles.get_mut(&key) {
                            Some(e) => {
                                e.last_used = frame;
                                let inst = tile_inst(origin, ratio, tx, ty, e.w, e.h, e.slot.layer);
                                let arr = e.slot.array;
                                self.push_image(2, arr, inst);
                            }
                            None => missing = true,
                        }
                    }
                }
            }
            // Fallback: the closest other scale we have for this page.
            if missing {
                if let Some(fs_bits) = self.index.best_fallback_scale(pi as u32, ts.to_bits()) {
                    let fs = f32::from_bits(fs_bits);
                    let fratio = cs / fs;
                    let fppx = tiles::page_px(g, fs);
                    if let Some(r) = tiles::tile_range(origin, fratio, fppx, cam.viewport, 0.0) {
                        for ty in r.ty0..=r.ty1 {
                            for tx in r.tx0..=r.tx1 {
                                let key = TileKey {
                                    page: pi as u32,
                                    scale: fs_bits,
                                    tx: tx as u16,
                                    ty: ty as u16,
                                };
                                if let Some(e) = self.index.tiles.get_mut(&key) {
                                    e.last_used = frame;
                                    let inst =
                                        tile_inst(origin, fratio, tx, ty, e.w, e.h, e.slot.layer);
                                    let arr = e.slot.array;
                                    self.push_image(1, arr, inst);
                                }
                            }
                        }
                    }
                }
            }
        }

        if !solids.is_empty() {
            self.batches.push(Batch {
                kind: BatchKind::Solid,
                first: 0,
                count: solids.len() as u32,
            });
            self.images.extend_from_slice(&solids);
        }
        let kinds: [(usize, KindMaker); 3] = [
            (0, BatchKind::Thumb),
            (1, BatchKind::Tile),
            (2, BatchKind::Tile),
        ];
        for (bucket, make) in kinds {
            for (arr, list) in self.buckets[bucket].iter().enumerate() {
                if list.is_empty() {
                    continue;
                }
                self.batches.push(Batch {
                    kind: make(arr as u16),
                    first: self.images.len() as u32,
                    count: list.len() as u32,
                });
                self.images.extend_from_slice(list);
            }
        }
    }

    fn live_instances(live: &LiveStroke, out: &mut Vec<StrokeInst>) {
        let s = Stroke {
            id: uuid::Uuid::nil(),
            page: live.page,
            points: live.points.to_vec(),
            pressure: live.pressure.map(|p| p.to_vec()),
            width: live.width,
            color: live.color,
            bbox: [0.0; 4],
        };
        Self::stroke_instances(&s, out);
    }

    pub fn render(&mut self, input: &FrameInput) -> FrameStats {
        let mut stats = FrameStats::default();
        let Acquired::Frame(frame) = self.gpu.acquire() else {
            return stats;
        };
        self.index.frame += 1;
        let [sw, sh] = self.gpu.size();
        let cam = input.camera;
        let dpr = cam.dpr;
        let cs = cam.scale();

        // --- globals ---
        let theme = input.theme;
        let dark_theme = recolor::DarkTheme::from_srgb8(theme.dark_fg, theme.dark_bg);
        let globals = Globals {
            viewport: [sw as f32, sh as f32],
            dark: theme.dark as u32,
            _pad: 0,
            fg: [dark_theme.fg[0], dark_theme.fg[1], dark_theme.fg[2], 0.0],
            bg: [dark_theme.bg[0], dark_theme.bg[1], dark_theme.bg[2], 0.0],
            fg_lin: [
                dark_theme.fg_lin[0],
                dark_theme.fg_lin[1],
                dark_theme.fg_lin[2],
                0.0,
            ],
            bg_lin: [
                dark_theme.bg_lin[0],
                dark_theme.bg_lin[1],
                dark_theme.bg_lin[2],
                0.0,
            ],
        };
        self.gpu
            .queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        let clear = if theme.dark {
            recolor::to_linear3(theme.dark_bg)
        } else {
            recolor::to_linear3([0xd6, 0xd6, 0xd6])
        };

        // --- page content ---
        self.build_images(input);
        let [_, y0, _, y1] = cam.visible_doc_rect();
        let vis = input.layout.visible(y0, y1);

        // Ink: sync GPU copies of visible pages, collect per-page uniforms.
        self.page_us.clear();
        let mut ink_draws: Vec<(usize, u32)> = Vec::new(); // (page, uniform index)
        for pi in vis.clone() {
            let g = &input.layout.pages[pi];
            let has_ink = !input.ink.strokes(pi).is_empty();
            let is_live = input.live.as_ref().map(|l| l.page == pi).unwrap_or(false);
            if !has_ink && !is_live {
                continue;
            }
            if has_ink {
                self.sync_page_ink(pi, input.ink);
            }
            let origin = tiles::page_origin(cam, g);
            ink_draws.push((pi, self.page_us.len() as u32));
            self.page_us.push(PageU {
                origin,
                scale: cs,
                _pad: 0.0,
            });
        }
        // Drop GPU ink of pages far away.
        let keep_lo = vis.start.saturating_sub(4);
        let keep_hi = vis.end + 4;
        self.page_ink.retain(|p, _| *p >= keep_lo && *p < keep_hi);

        let stride = self.page_u_stride as usize;
        if !self.page_us.is_empty() {
            let need = (self.page_us.len() * stride) as u64;
            if need > self.page_u_buf.cap {
                self.page_u_buf = GrowBuf::new(
                    &self.gpu.device,
                    "page-uniforms",
                    wgpu::BufferUsages::UNIFORM,
                    need.next_power_of_two(),
                );
                self.page_u_bg =
                    Self::make_page_u_bg(&self.gpu.device, &self.page_u_bgl, &self.page_u_buf);
            }
            let mut bytes = vec![0u8; self.page_us.len() * stride];
            for (i, u) in self.page_us.iter().enumerate() {
                bytes[i * stride..i * stride + 16].copy_from_slice(bytemuck::bytes_of(u));
            }
            self.gpu.queue.write_buffer(&self.page_u_buf.buf, 0, &bytes);
        }

        // Live stroke
        let mut live_count = 0u32;
        let mut live_uniform = None;
        if let Some(live) = &input.live {
            self.stroke_scratch.clear();
            Self::live_instances(live, &mut self.stroke_scratch);
            live_count = self.stroke_scratch.len() as u32;
            self.live_buf.write(
                &self.gpu.device,
                &self.gpu.queue,
                bytemuck::cast_slice(&self.stroke_scratch),
            );
            live_uniform = ink_draws
                .iter()
                .find(|(p, _)| *p == live.page)
                .map(|(_, u)| *u);
        }

        // --- overlay rectangles ---
        self.rects_page.clear();
        self.rects_hl.clear();
        self.rects_ui.clear();
        if theme.dark {
            if let Some(sep) = theme.separator {
                let c = recolor::to_linear3(sep);
                for pi in vis.clone() {
                    let g = &input.layout.pages[pi];
                    let o = tiles::page_origin(cam, g);
                    let y = o[1] + g.h * cs + (crate::view::layout::PAGE_GAP * cs * 0.5).floor();
                    self.rects_page.push(OverlayInst::rect(
                        0.0,
                        y,
                        sw as f32,
                        1.0f32.max(dpr.floor()),
                        [c[0], c[1], c[2], 1.0],
                    ));
                }
            }
        }
        for h in input.highlights {
            if h.page < vis.start || h.page >= vis.end {
                continue;
            }
            let g = &input.layout.pages[h.page];
            let o = tiles::page_origin(cam, g);
            let r = h.rect;
            // Light pages: multiply (text stays dark). Dark pages: plain tint.
            let col = match (theme.dark, h.current) {
                (false, true) => [1.0, 0.45, 0.1, 1.0],
                (false, false) => [1.0, 0.92, 0.35, 1.0],
                (true, true) => [0.9, 0.45, 0.0, 0.55],
                (true, false) => [0.8, 0.7, 0.0, 0.4],
            };
            self.rects_hl.push(OverlayInst::rect(
                o[0] + r[0] * cs,
                o[1] + r[1] * cs,
                (r[2] - r[0]) * cs,
                (r[3] - r[1]) * cs,
                col,
            ));
        }
        if let Some(c) = &input.cursor {
            let col = if c.eraser {
                if theme.dark {
                    [1.0, 1.0, 1.0, 0.9]
                } else {
                    [0.0, 0.0, 0.0, 0.9]
                }
            } else {
                let l = recolor::to_linear3(c.color);
                [l[0], l[1], l[2], 0.9]
            };
            self.rects_page.push(OverlayInst::ring(
                c.pos[0],
                c.pos[1],
                c.radius.max(2.0),
                1.5 * dpr,
                col,
            ));
        }
        let ui_state = input.ui;
        self.ui.update(
            &self.gpu.device,
            &self.gpu.queue,
            ui_state,
            [sw, sh],
            dpr,
            &mut self.rects_ui,
        );
        let bar_h = self.ui.bar_height;
        let page_rect_count = self.rects_page.len() as u32;
        let hl_count = self.rects_hl.len() as u32;
        let mut all_rects: Vec<OverlayInst> =
            Vec::with_capacity(self.rects_page.len() + self.rects_hl.len() + self.rects_ui.len());
        all_rects.extend_from_slice(&self.rects_page);
        all_rects.extend_from_slice(&self.rects_hl);
        all_rects.extend_from_slice(&self.rects_ui);

        self.image_buf.write(
            &self.gpu.device,
            &self.gpu.queue,
            bytemuck::cast_slice(&self.images),
        );
        self.overlay_buf.write(
            &self.gpu.device,
            &self.gpu.queue,
            bytemuck::cast_slice(&all_rects),
        );

        // --- draw ---
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let page_h = (sh as f32 - bar_h).max(1.0) as u32;
            pass.set_scissor_rect(0, 0, sw, page_h.min(sh));

            // Pages
            if !self.batches.is_empty() {
                pass.set_pipeline(&self.image_pipe);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_vertex_buffer(0, self.image_buf.buf.slice(..));
                let mut bound: Option<(bool, u16)> = None;
                for b in &self.batches {
                    let (is_thumb, arr) = match b.kind {
                        BatchKind::Solid => {
                            // Any texture will do; use the first tile array or preview array.
                            if self.tile_arrays.array_count() > 0 {
                                (false, 0)
                            } else if self.thumb_arrays.array_count() > 0 {
                                (true, 0)
                            } else {
                                // No texture exists yet: nothing but blank pages to show.
                                // Create the first tile array so there is something to bind.
                                (false, u16::MAX)
                            }
                        }
                        BatchKind::Thumb(a) => (true, a),
                        BatchKind::Tile(a) => (false, a),
                    };
                    if arr == u16::MAX {
                        continue;
                    }
                    if bound != Some((is_thumb, arr)) {
                        let bg = if is_thumb {
                            self.thumb_arrays.bind_group(arr)
                        } else {
                            self.tile_arrays.bind_group(arr)
                        };
                        pass.set_bind_group(1, bg, &[]);
                        bound = Some((is_thumb, arr));
                    }
                    pass.draw(0..6, b.first..b.first + b.count);
                    stats.draw_calls += 1;
                    stats.image_instances += b.count;
                }
            }

            // Ink
            let any_stroke = ink_draws
                .iter()
                .any(|(p, _)| self.page_ink.get(p).map(|i| i.count > 0).unwrap_or(false))
                || live_count > 0;
            if any_stroke {
                pass.set_pipeline(&self.stroke_pipe);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                for (pi, ui_idx) in &ink_draws {
                    if let Some(ink) = self.page_ink.get(pi) {
                        if ink.count > 0 {
                            pass.set_bind_group(
                                1,
                                &self.page_u_bg,
                                &[(*ui_idx as u64 * self.page_u_stride) as u32],
                            );
                            pass.set_vertex_buffer(0, ink.buf.buf.slice(..));
                            pass.draw(0..6, 0..ink.count);
                            stats.draw_calls += 1;
                            stats.stroke_instances += ink.count;
                        }
                    }
                }
                if let (Some(u), true) = (live_uniform, live_count > 0) {
                    pass.set_bind_group(
                        1,
                        &self.page_u_bg,
                        &[(u as u64 * self.page_u_stride) as u32],
                    );
                    pass.set_vertex_buffer(0, self.live_buf.buf.slice(..));
                    pass.draw(0..6, 0..live_count);
                    stats.draw_calls += 1;
                    stats.stroke_instances += live_count;
                }
            }

            // Highlights, cursor
            if !all_rects.is_empty() {
                pass.set_pipeline(&self.overlay_pipe);
                pass.set_bind_group(0, &self.globals_bg, &[]);
                pass.set_vertex_buffer(0, self.overlay_buf.buf.slice(..));
                if hl_count > 0 {
                    if !theme.dark {
                        pass.set_pipeline(&self.multiply_pipe);
                        pass.set_bind_group(0, &self.globals_bg, &[]);
                        pass.set_vertex_buffer(0, self.overlay_buf.buf.slice(..));
                    }
                    pass.draw(0..6, page_rect_count..page_rect_count + hl_count);
                    stats.draw_calls += 1;
                    if !theme.dark {
                        pass.set_pipeline(&self.overlay_pipe);
                        pass.set_bind_group(0, &self.globals_bg, &[]);
                        pass.set_vertex_buffer(0, self.overlay_buf.buf.slice(..));
                    }
                }
                if page_rect_count > 0 {
                    pass.draw(0..6, 0..page_rect_count);
                    stats.draw_calls += 1;
                }
                pass.set_scissor_rect(0, 0, sw, sh);
                let ui_first = page_rect_count + hl_count;
                if all_rects.len() as u32 > ui_first {
                    pass.draw(0..6, ui_first..all_rects.len() as u32);
                    stats.draw_calls += 1;
                }
            }
            pass.set_scissor_rect(0, 0, sw, sh);
            self.ui.render(&mut pass);
            stats.draw_calls += 1;
        }
        let readback = if std::mem::take(&mut self.capture_requested)
            && self
                .gpu
                .config
                .usage
                .contains(wgpu::TextureUsages::COPY_SRC)
        {
            let bpr = (sw * 4).div_ceil(256) * 256;
            let buf = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("capture"),
                size: (bpr * sh) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &frame.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buf,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bpr),
                        rows_per_image: Some(sh),
                    },
                },
                wgpu::Extent3d {
                    width: sw,
                    height: sh,
                    depth_or_array_layers: 1,
                },
            );
            Some((buf, bpr))
        } else {
            None
        };
        self.gpu.queue.submit(Some(encoder.finish()));
        if let Some((buf, bpr)) = readback {
            let slice = buf.slice(..);
            slice.map_async(wgpu::MapMode::Read, |_| {});
            let _ = self.gpu.device.poll(wgpu::PollType::wait_indefinitely());
            let data = slice.get_mapped_range().expect("capture buffer is mapped");
            let bgra = matches!(
                self.gpu.format(),
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
            );
            let mut rgba = Vec::with_capacity((sw * sh * 4) as usize);
            for row in 0..sh as usize {
                let line = &data[row * bpr as usize..row * bpr as usize + sw as usize * 4];
                if bgra {
                    for px in line.chunks_exact(4) {
                        rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
                    }
                } else {
                    rgba.extend_from_slice(line);
                }
            }
            drop(data);
            buf.unmap();
            self.captured = Some(Capture {
                width: sw,
                height: sh,
                rgba,
            });
        }
        self.gpu.queue.present(frame);
        self.ui.end_frame();

        stats.tiles_cached = self.index.tiles.len() as u32;
        stats.tile_slots = self.tile_arrays.in_use() as u32;
        stats
    }
}

fn tile_inst(
    origin: [f32; 2],
    ratio: f32,
    tx: u32,
    ty: u32,
    w: u16,
    h: u16,
    layer: u16,
) -> ImageInst {
    let t = TILE as f32;
    ImageInst {
        rect: [
            origin[0] + tx as f32 * t * ratio,
            origin[1] + ty as f32 * t * ratio,
            w as f32 * ratio,
            h as f32 * ratio,
        ],
        uv: [0.0, 0.0, w as f32 / t, h as f32 / t],
        layer: layer as u32,
        _pad: [0; 3],
    }
}
