//! Status line, command line and list overlay, drawn with glyphon.
//!
//! The only font is the one embedded in the binary: system font lookup is
//! slow at startup and unreliable on NixOS. System fonts are loaded lazily,
//! once, only when a string contains characters the embedded font lacks.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

use super::overlay::OverlayInst;

const FONT: &[u8] = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
const FONT_NAME: &str = "DejaVu Sans Mono";
/// Advance width of the embedded monospace font, in em.
const ADVANCE: f32 = 0.602;
const FONT_SIZE: f32 = 13.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListOverlay {
    pub title: String,
    pub lines: Vec<String>,
    pub selected: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiState {
    pub statusbar: bool,
    pub left: String,
    pub left_is_error: bool,
    pub right: String,
    /// Colour and text of the pen indicator in the status bar.
    pub pen: Option<([u8; 3], String)>,
    /// `:` / `/` input line, shown instead of `left`.
    pub input: Option<String>,
    pub overlay: Option<ListOverlay>,
    pub hint: Option<String>,
    pub dark: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct Layout {
    w: u32,
    h: u32,
    dpr: f32,
}

pub struct Ui {
    font_system: FontSystem,
    swash: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    left: Buffer,
    right: Buffer,
    pen: Buffer,
    title: Buffer,
    list: Buffer,
    hint: Buffer,
    shown: UiState,
    shown_layout: Option<Layout>,
    system_fonts_loaded: bool,
    prepared_this_frame: bool,
    // Geometry computed in `prepare`.
    areas: Vec<Area>,
    pub bar_height: f32,
}

#[derive(Clone, Copy)]
enum Which {
    Left,
    Right,
    Pen,
    Title,
    List,
    Hint,
}

struct Area {
    which: Which,
    left: f32,
    top: f32,
    bounds: [i32; 4],
    color: [u8; 3],
}

/// Scripts and symbols DejaVu Sans Mono has glyphs for: Latin, Greek,
/// Cyrillic, general punctuation, arrows, maths, box drawing and blocks.
/// Anything else (CJK, Arabic, Hebrew, Thai, ...) needs system fonts.
fn embedded_font_covers(c: char) -> bool {
    let u = c as u32;
    u <= 0x052F
        || (0x1E00..=0x1FFF).contains(&u)
        || (0x2000..=0x2BFF).contains(&u)
        || (0xFB00..=0xFB06).contains(&u)
}

fn attrs() -> Attrs<'static> {
    Attrs::new().family(Family::Name(FONT_NAME))
}

impl Ui {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Ui {
        let mut font_system = FontSystem::new_with_fonts([glyphon::fontdb::Source::Binary(
            std::sync::Arc::new(FONT),
        )]);
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let m = Metrics::new(FONT_SIZE, FONT_SIZE * 1.4);
        let mk = |fs: &mut FontSystem| Buffer::new(fs, m);
        let left = mk(&mut font_system);
        let right = mk(&mut font_system);
        let pen = mk(&mut font_system);
        let title = mk(&mut font_system);
        let list = mk(&mut font_system);
        let hint = mk(&mut font_system);
        Ui {
            font_system,
            swash: SwashCache::new(),
            viewport,
            atlas,
            renderer,
            left,
            right,
            pen,
            title,
            list,
            hint,
            shown: UiState::default(),
            shown_layout: None,
            system_fonts_loaded: false,
            prepared_this_frame: false,
            areas: Vec::new(),
            bar_height: 0.0,
        }
    }

    fn set_text(
        &mut self,
        which: Which,
        text: &str,
        w: Option<f32>,
        h: Option<f32>,
        metrics: Metrics,
    ) {
        if !self.system_fonts_loaded
            && text
                .chars()
                .any(|c| !c.is_whitespace() && !embedded_font_covers(c))
        {
            self.font_system.db_mut().load_system_fonts();
            self.system_fonts_loaded = true;
        }
        let buf = match which {
            Which::Left => &mut self.left,
            Which::Right => &mut self.right,
            Which::Pen => &mut self.pen,
            Which::Title => &mut self.title,
            Which::List => &mut self.list,
            Which::Hint => &mut self.hint,
        };
        buf.set_metrics(metrics);
        buf.set_size(w, h);
        buf.set_text(text, &attrs(), Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.font_system, false);
    }

    /// Height of the status bar in physical pixels.
    pub fn bar_height_for(dpr: f32) -> f32 {
        (FONT_SIZE * 1.4 * dpr + 8.0 * dpr).ceil()
    }

    /// Lay out text for `state`; pushes the background rectangles that belong
    /// to the UI into `rects`. Returns whether anything has to be re-prepared.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        state: &UiState,
        size: [u32; 2],
        dpr: f32,
        rects: &mut Vec<OverlayInst>,
    ) {
        let (w, h) = (size[0] as f32, size[1] as f32);
        let layout = Layout {
            w: size[0],
            h: size[1],
            dpr,
        };
        self.bar_height = if state.statusbar || state.input.is_some() {
            Self::bar_height_for(dpr)
        } else {
            0.0
        };
        let (bg, fg, dim, err, panel, sel) = if state.dark {
            (
                [0.0; 3],
                [255, 255, 255],
                [138, 138, 138],
                [255, 95, 95],
                [0.04, 0.04, 0.04],
                [0.16, 0.16, 0.18],
            )
        } else {
            (
                [1.0; 3],
                [0, 0, 0],
                [110, 110, 110],
                [200, 40, 40],
                [0.97, 0.97, 0.97],
                [0.82, 0.86, 0.94],
            )
        };
        let to_lin = |c: [f32; 3]| [lin(c[0]), lin(c[1]), lin(c[2])];

        let changed = self.shown != *state || self.shown_layout != Some(layout);
        let px = FONT_SIZE * dpr;
        let cw = px * ADVANCE;
        let metrics = Metrics::new(px, px * 1.4);
        let line_h = px * 1.4;
        let pad = 8.0 * dpr;

        if changed {
            self.areas.clear();
            let bar_top = h - self.bar_height;
            let text_top = bar_top + (self.bar_height - line_h) * 0.5;

            // Right side: pen swatch + pen text + info, laid out from the right.
            let mut x_right = w - pad;
            if self.bar_height > 0.0 && state.input.is_none() {
                let info_w = state.right.chars().count() as f32 * cw;
                x_right -= info_w;
                self.set_text(
                    Which::Right,
                    &state.right,
                    Some(info_w + cw),
                    Some(line_h * 1.5),
                    metrics,
                );
                self.areas.push(Area {
                    which: Which::Right,
                    left: x_right,
                    top: text_top,
                    bounds: [0, bar_top as i32, w as i32, h as i32],
                    color: dim,
                });
                if let Some((_, text)) = &state.pen {
                    x_right -= 2.0 * cw;
                    let pen_w = text.chars().count() as f32 * cw;
                    x_right -= pen_w;
                    self.set_text(
                        Which::Pen,
                        text,
                        Some(pen_w + cw),
                        Some(line_h * 1.5),
                        metrics,
                    );
                    self.areas.push(Area {
                        which: Which::Pen,
                        left: x_right,
                        top: text_top,
                        bounds: [0, bar_top as i32, w as i32, h as i32],
                        color: fg,
                    });
                }
            }
            // Left side: input line or message / status.
            let left_text = state.input.clone().unwrap_or_else(|| state.left.clone());
            if self.bar_height > 0.0 || !left_text.is_empty() {
                let max_w = (x_right - pad * 2.0).max(cw);
                self.set_text(
                    Which::Left,
                    &left_text,
                    Some(max_w + cw),
                    Some(line_h * 1.5),
                    metrics,
                );
                let color = if state.left_is_error && state.input.is_none() {
                    err
                } else {
                    fg
                };
                self.areas.push(Area {
                    which: Which::Left,
                    left: pad,
                    top: text_top,
                    bounds: [0, bar_top as i32, x_right.max(cw) as i32, h as i32],
                    color,
                });
            }
            if let Some(hint) = &state.hint {
                let hw = hint.chars().count() as f32 * cw;
                self.set_text(
                    Which::Hint,
                    hint,
                    Some(hw + cw),
                    Some(line_h * 1.5),
                    metrics,
                );
                self.areas.push(Area {
                    which: Which::Hint,
                    left: ((w - hw) * 0.5).max(0.0),
                    top: (h - self.bar_height) * 0.5 - line_h,
                    bounds: [0, 0, w as i32, h as i32],
                    color: dim,
                });
            }
            if let Some(ov) = &state.overlay {
                let pw = (w * 0.7).min(w - 2.0 * pad).max(cw * 10.0);
                let rows = ((h * 0.7 / line_h) as usize).saturating_sub(2).max(3);
                let shown = ov.lines.len().min(rows);
                // Keep the selection inside the visible window.
                let first = if ov.selected >= shown {
                    ov.selected + 1 - shown
                } else {
                    0
                };
                let ph = line_h * (shown as f32 + 2.0) + pad;
                let px0 = (w - pw) * 0.5;
                let py0 = ((h - self.bar_height - ph) * 0.4).max(pad);
                let _ = (px0, py0);
                let text: String = ov.lines[first..first + shown].join("\n");
                self.set_text(
                    Which::Title,
                    &ov.title,
                    Some(pw),
                    Some(line_h * 1.5),
                    metrics,
                );
                self.set_text(
                    Which::List,
                    &text,
                    Some(pw - 2.0 * pad),
                    Some(line_h * (shown as f32 + 1.0)),
                    metrics,
                );
                self.areas.push(Area {
                    which: Which::Title,
                    left: px0 + pad,
                    top: py0 + pad * 0.5,
                    bounds: [px0 as i32, py0 as i32, (px0 + pw) as i32, (py0 + ph) as i32],
                    color: dim,
                });
                self.areas.push(Area {
                    which: Which::List,
                    left: px0 + pad,
                    top: py0 + pad * 0.5 + line_h * 1.4,
                    bounds: [px0 as i32, py0 as i32, (px0 + pw) as i32, (py0 + ph) as i32],
                    color: fg,
                });
            }
            self.shown = state.clone();
            self.shown_layout = Some(layout);

            self.viewport.update(
                queue,
                Resolution {
                    width: size[0],
                    height: size[1],
                },
            );
            let text_areas: Vec<TextArea<'_>> = self
                .areas
                .iter()
                .map(|a| {
                    let buffer = match a.which {
                        Which::Left => &self.left,
                        Which::Right => &self.right,
                        Which::Pen => &self.pen,
                        Which::Title => &self.title,
                        Which::List => &self.list,
                        Which::Hint => &self.hint,
                    };
                    TextArea {
                        buffer,
                        left: a.left,
                        top: a.top,
                        scale: 1.0,
                        bounds: TextBounds {
                            left: a.bounds[0],
                            top: a.bounds[1],
                            right: a.bounds[2],
                            bottom: a.bounds[3],
                        },
                        default_color: Color::rgb(a.color[0], a.color[1], a.color[2]),
                        custom_glyphs: &[],
                    }
                })
                .collect();
            self.prepared_this_frame = true;
            if let Err(e) = self.renderer.prepare(
                device,
                queue,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                text_areas,
                &mut self.swash,
            ) {
                log::warn!("text prepare: {e}");
            }
        }

        // Rectangles are cheap; rebuild every call so the renderer's list is complete.
        if self.bar_height > 0.0 {
            let c = to_lin(bg);
            rects.push(OverlayInst::rect(
                0.0,
                h - self.bar_height,
                w,
                self.bar_height,
                [c[0], c[1], c[2], 1.0],
            ));
            if let (Some((color, _)), None) = (&state.pen, &state.input) {
                // Swatch left of the pen text.
                let info_w = state.right.chars().count() as f32 * cw;
                let pen_w = state
                    .pen
                    .as_ref()
                    .map(|(_, t)| t.chars().count())
                    .unwrap_or(0) as f32
                    * cw;
                let x = w - pad - info_w - 2.0 * cw - pen_w;
                let d = line_h * 0.55;
                let cy = h - self.bar_height * 0.5;
                let l = [
                    lin(color[0] as f32 / 255.0),
                    lin(color[1] as f32 / 255.0),
                    lin(color[2] as f32 / 255.0),
                ];
                rects.push(OverlayInst::disc(
                    x - d - cw * 0.4,
                    cy - d * 0.5,
                    d,
                    [l[0], l[1], l[2], 1.0],
                ));
            }
        }
        if let Some(ov) = &state.overlay {
            let pw = (w * 0.7).min(w - 2.0 * pad).max(cw * 10.0);
            let rows = ((h * 0.7 / line_h) as usize).saturating_sub(2).max(3);
            let shown = ov.lines.len().min(rows);
            let first = if ov.selected >= shown {
                ov.selected + 1 - shown
            } else {
                0
            };
            let ph = line_h * (shown as f32 + 2.0) + pad;
            let px0 = (w - pw) * 0.5;
            let py0 = ((h - self.bar_height - ph) * 0.4).max(pad);
            let p = to_lin(panel);
            rects.push(OverlayInst::rect(
                px0,
                py0,
                pw,
                ph,
                [p[0], p[1], p[2], 0.97],
            ));
            if ov.selected >= first && ov.selected < first + shown {
                let s = to_lin(sel);
                let y = py0 + pad * 0.5 + line_h * 1.4 + (ov.selected - first) as f32 * line_h;
                rects.push(OverlayInst::rect(
                    px0 + pad * 0.5,
                    y,
                    pw - pad,
                    line_h,
                    [s[0], s[1], s[2], 1.0],
                ));
            }
        }
    }

    pub fn render<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) {
        if self.areas.is_empty() {
            return;
        }
        if let Err(e) = self.renderer.render(&self.atlas, &self.viewport, pass) {
            log::warn!("text render: {e}");
        }
    }

    pub fn end_frame(&mut self) {
        // Only trim after a prepare: otherwise glyphs that the still-valid
        // prepared geometry refers to would be evicted.
        if std::mem::take(&mut self.prepared_this_frame) {
            self.atlas.trim();
        }
    }
}

fn lin(c: f32) -> f32 {
    super::recolor::srgb_to_linear(c)
}
