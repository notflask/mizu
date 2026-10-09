//! The window: translates winit events into viewer calls and drives frames.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::StartCause;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowId};

use crate::config::Settings;
use crate::input::keys::parse_seq;
use crate::input::Key;
use crate::platform::{self, PlatformEvent};
use crate::render::{FrameInput, Renderer};
use crate::viewer::input::{Button, Wheel};
use crate::viewer::{LoadPurpose, UiMode, Viewer};

pub enum UserEvent {
    /// A background thread has something for us.
    Wake,
    Platform(PlatformEvent),
}

pub struct Options {
    pub file: Option<PathBuf>,
    pub page: Option<usize>,
    pub stats: bool,
    pub diag: bool,
}

pub struct App {
    viewer: Viewer,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    proxy: EventLoopProxy<UserEvent>,
    mods: ModifiersState,
    title: String,
    cache_epoch: u64,
    stats: FrameTimes,
    script: Option<Script>,
    capture_enabled: bool,
    platform: Option<platform::Platform>,
    pinch_pos: Option<[f32; 2]>,
    touch_id: Option<u64>,
    pub exit_error: Option<String>,
    diag: Option<Diag>,
    presented_once: bool,
}

/// `--diag`: reads back some frames and logs what they contain.
struct Diag {
    frames: u32,
    last: Instant,
    want: bool,
}

impl Diag {
    fn before(&mut self, r: &mut Renderer, now: Instant) {
        self.want = self.frames < 8 || now.duration_since(self.last) > Duration::from_secs(3);
        if self.want {
            r.request_capture();
        }
    }

    fn after(&mut self, r: &mut Renderer, fs: &crate::render::FrameStats, now: Instant) {
        self.frames += 1;
        if !std::mem::take(&mut self.want) {
            return;
        }
        self.last = now;
        let head = format!(
            "diag frame {}: draw calls {}, page quads {}, stroke quads {}, tiles {}/{}, skipped {:?}",
            self.frames,
            fs.draw_calls,
            fs.image_instances,
            fs.stroke_instances,
            fs.tiles_cached,
            fs.tile_slots,
            fs.skipped
        );
        match r.take_capture() {
            Some(cap) => log::info!(
                "{head}\n{}",
                crate::render::diag::describe(&cap, r.bar_height().ceil() as u32)
            ),
            None => log::info!("{head}\n  (nothing read back)"),
        }
    }
}

// ----------------------------------------------------------------------
// frame statistics (`--stats`)
// ----------------------------------------------------------------------

struct FrameTimes {
    samples: VecDeque<(Instant, f32)>,
}

impl FrameTimes {
    fn push(&mut self, now: Instant, ms: f32) {
        self.samples.push_back((now, ms));
        while self
            .samples
            .front()
            .map(|(t, _)| now.duration_since(*t) > Duration::from_secs(1))
            .unwrap_or(false)
        {
            self.samples.pop_front();
        }
    }

    fn summary(&self) -> (f32, f32) {
        if self.samples.is_empty() {
            return (0.0, 0.0);
        }
        let sum: f32 = self.samples.iter().map(|(_, m)| m).sum();
        let max = self.samples.iter().map(|(_, m)| *m).fold(0.0, f32::max);
        (sum / self.samples.len() as f32, max)
    }
}

// ----------------------------------------------------------------------
// test driver: MIZU_TEST_SCRIPT=<file> runs commands against the real window
// ----------------------------------------------------------------------

enum Cmd {
    Keys(Vec<Key>),
    Move(f32, f32),
    Button(Button, bool),
    Wheel(f32, f32, bool),
    Pinch(f32),
    Settle,
    Sleep(Duration),
    Capture(PathBuf),
    Dump,
    Quit,
}

struct Script {
    cmds: VecDeque<Cmd>,
    sleeping_until: Option<Instant>,
}

impl Script {
    fn load(path: &str) -> Result<Script, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let mut cmds = VecDeque::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
            let num = |s: &str| {
                s.parse::<f32>()
                    .map_err(|_| format!("line {}: bad number {s:?}", n + 1))
            };
            let cmd = match op {
                "key" | "type" => {
                    Cmd::Keys(parse_seq(rest).map_err(|e| format!("line {}: {e}", n + 1))?)
                }
                "move" => {
                    let mut it = rest.split_whitespace();
                    Cmd::Move(num(it.next().unwrap_or(""))?, num(it.next().unwrap_or(""))?)
                }
                "down" | "up" => {
                    let b = match rest {
                        "left" => Button::Left,
                        "middle" => Button::Middle,
                        "right" => Button::Right,
                        _ => return Err(format!("line {}: unknown button", n + 1)),
                    };
                    Cmd::Button(b, op == "down")
                }
                "wheel" => {
                    let mut it = rest.split_whitespace();
                    let x = num(it.next().unwrap_or(""))?;
                    let y = num(it.next().unwrap_or(""))?;
                    Cmd::Wheel(x, y, it.next() == Some("ctrl"))
                }
                "pinch" => Cmd::Pinch(num(rest)?),
                "settle" => Cmd::Settle,
                "sleep" => Cmd::Sleep(Duration::from_millis(num(rest)? as u64)),
                "capture" => Cmd::Capture(PathBuf::from(rest)),
                "dump" => Cmd::Dump,
                "quit" => Cmd::Quit,
                _ => return Err(format!("line {}: unknown command {op:?}", n + 1)),
            };
            cmds.push_back(cmd);
        }
        Ok(Script {
            cmds,
            sleeping_until: None,
        })
    }
}

fn write_ppm(path: &std::path::Path, c: &crate::render::Capture) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(f, "P6\n{} {}\n255\n", c.width, c.height)?;
    let mut rgb = Vec::with_capacity((c.width * c.height * 3) as usize);
    for px in c.rgba.as_chunks::<4>().0 {
        rgb.extend_from_slice(&px[..3]);
    }
    f.write_all(&rgb)
}

impl App {
    pub fn new(opts: Options, proxy: EventLoopProxy<UserEvent>) -> App {
        let (settings, warning): (Settings, Option<String>) = Settings::load();
        let p = proxy.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = p.send_event(UserEvent::Wake);
        });
        let mut viewer = Viewer::new(settings, warning, wake);
        viewer.show_stats = opts.stats;
        if let Some(f) = opts.file {
            viewer.open(f, LoadPurpose::Open { page: opts.page });
        }
        let mut exit_error = None;
        let script = match std::env::var("MIZU_TEST_SCRIPT") {
            Ok(path) => match Script::load(&path) {
                Ok(s) => Some(s),
                Err(e) => {
                    exit_error = Some(e);
                    None
                }
            },
            Err(_) => None,
        };
        let diag = (opts.diag || std::env::var_os("MIZU_DIAG").is_some()).then(|| Diag {
            frames: 0,
            last: Instant::now(),
            want: false,
        });
        App {
            viewer,
            renderer: None,
            window: None,
            proxy,
            mods: ModifiersState::empty(),
            title: String::new(),
            cache_epoch: 0,
            stats: FrameTimes {
                samples: VecDeque::new(),
            },
            capture_enabled: script.is_some() || diag.is_some(),
            diag,
            presented_once: false,
            script,
            platform: None,
            pinch_pos: None,
            touch_id: None,
            exit_error,
        }
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn sync_window_size(&mut self) {
        let (Some(w), Some(r)) = (&self.window, &mut self.renderer) else {
            return;
        };
        let size = w.inner_size();
        r.resize(size.width, size.height);
        self.viewer
            .set_window([size.width, size.height], w.scale_factor() as f32);
    }

    fn handle_platform(&mut self, ev: PlatformEvent) {
        // Backends report logical pixels; the viewer works in physical ones.
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor() as f32)
            .unwrap_or(1.0);
        let phys = |p: [f32; 2]| [p[0] * scale, p[1] * scale];
        match ev {
            PlatformEvent::PinchBegin { pos } => self.pinch_pos = Some(phys(pos)),
            PlatformEvent::PinchUpdate { delta, pos } => {
                let pos = phys(pos);
                self.pinch_pos = Some(pos);
                self.viewer.on_pinch(delta, Some(pos));
            }
            PlatformEvent::PinchEnd => self.pinch_pos = None,
            PlatformEvent::PenHover { pos } => self.viewer.on_cursor_moved(phys(pos)),
            PlatformEvent::PenDown {
                pos,
                pressure,
                eraser,
            } => {
                let pos = phys(pos);
                self.viewer.mouse.pos = pos;
                self.viewer.mouse.inside = true;
                if self.viewer.mode == UiMode::Draw {
                    let erase = eraser || self.viewer.tool == crate::viewer::Tool::Eraser;
                    self.viewer.pen_down(pos, Some(pressure), erase);
                } else {
                    self.viewer.on_mouse_button(Button::Left, true);
                }
            }
            PlatformEvent::PenMove { pos, pressure } => {
                let pos = phys(pos);
                if self.viewer.pen.is_active() {
                    self.viewer.mouse.pos = pos;
                    self.viewer.pen_move(pos, Some(pressure));
                } else {
                    self.viewer.on_cursor_moved(pos);
                }
            }
            PlatformEvent::PenUp => {
                if self.viewer.pen.is_active() {
                    self.viewer.pen_up();
                } else {
                    self.viewer.on_mouse_button(Button::Left, false);
                }
            }
        }
        self.viewer.dirty = true;
    }

    fn script_step(&mut self, event_loop: &ActiveEventLoop) {
        let Some(mut script) = self.script.take() else {
            return;
        };
        loop {
            if let Some(t) = script.sleeping_until {
                if Instant::now() < t {
                    break;
                }
                script.sleeping_until = None;
            }
            let Some(cmd) = script.cmds.front() else {
                event_loop.exit();
                break;
            };
            match cmd {
                Cmd::Settle => {
                    let ready = self
                        .renderer
                        .as_ref()
                        .map(|r| self.viewer.view_complete(r) && !self.viewer.dirty)
                        .unwrap_or(false);
                    if !ready {
                        break;
                    }
                }
                Cmd::Sleep(d) => {
                    script.sleeping_until = Some(Instant::now() + *d);
                }
                Cmd::Capture(_) => {
                    // Needs a freshly rendered frame: request it, then wait for it.
                    let Some(r) = &mut self.renderer else { break };
                    if let Some(cap) = r.take_capture() {
                        if let Some(Cmd::Capture(path)) = script.cmds.pop_front() {
                            if let Err(e) = write_ppm(&path, &cap) {
                                self.exit_error = Some(format!("capture: {e}"));
                                event_loop.exit();
                            }
                        }
                        continue;
                    }
                    r.request_capture();
                    self.viewer.dirty = true;
                    self.redraw();
                    break;
                }
                _ => {}
            }
            let input_cmd = matches!(
                script.cmds.front(),
                Some(
                    Cmd::Keys(_) | Cmd::Move(..) | Cmd::Button(..) | Cmd::Wheel(..) | Cmd::Pinch(_)
                )
            );
            match script.cmds.pop_front() {
                Some(Cmd::Keys(keys)) => {
                    for k in keys {
                        self.viewer.on_key(k);
                    }
                }
                Some(Cmd::Move(x, y)) => self.viewer.on_cursor_moved([x, y]),
                Some(Cmd::Button(b, down)) => self.viewer.on_mouse_button(b, down),
                Some(Cmd::Wheel(x, y, ctrl)) => self.viewer.on_wheel(Wheel::Lines(x, y), ctrl),
                Some(Cmd::Pinch(d)) => self.viewer.on_pinch(d, None),
                Some(Cmd::Dump) => {
                    let v = &self.viewer;
                    if let Some(r) = &self.renderer {
                        eprintln!(
                            "COMPLETE={} loading={} dirty={}",
                            v.view_complete(r),
                            v.is_loading(),
                            v.dirty
                        );
                    }
                    eprintln!(
                        "STATE page={}/{} zoom={:.3} mode={:?} dark={} dirty={} strokes={} offset=({:.1},{:.1}) msg={:?}",
                        v.current_page() + 1,
                        v.page_count(),
                        v.camera.zoom,
                        v.mode,
                        v.dark,
                        v.is_dirty(),
                        v.doc.as_ref().map(|d| d.ink.total()).unwrap_or(0),
                        v.camera.offset[0],
                        v.camera.offset[1],
                        v.message.as_ref().map(|m| m.text.clone()),
                    );
                }
                Some(Cmd::Quit) => {
                    event_loop.exit();
                    break;
                }
                _ => {}
            }
            if input_cmd {
                // Only real input forces a redraw; everything else has to wake
                // the drawing code by itself, exactly as in normal use.
                self.viewer.dirty = true;
                self.redraw();
            }
        }
        self.script = Some(script);
    }
}

fn touch_pressure(f: &Option<winit::event::Force>) -> Option<f32> {
    f.map(|f| f.normalized() as f32)
        .filter(|p| p.is_finite() && *p > 0.0)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let mut attrs = Window::default_attributes()
            .with_title(self.viewer.title())
            .with_inner_size(LogicalSize::new(1100.0, 820.0))
            .with_min_inner_size(LogicalSize::new(320.0, 240.0));
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = WindowAttributesExtWayland::with_name(attrs, crate::config::APP_ID, "mizu");
            attrs = WindowAttributesExtX11::with_name(attrs, "mizu", crate::config::APP_ID);
        }
        if let Some(icon) = crate::icon::window_icon() {
            attrs = attrs.with_window_icon(Some(icon));
        }
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.exit_error = Some(format!("cannot create a window: {e}"));
                event_loop.exit();
                return;
            }
        };
        let renderer = match Renderer::new(
            window.clone(),
            event_loop,
            self.viewer.settings.tile_cache_mb,
            self.capture_enabled,
        ) {
            Ok(r) => r,
            Err(e) => {
                self.exit_error = Some(format!("{e:#}"));
                event_loop.exit();
                return;
            }
        };
        self.renderer = Some(renderer);
        self.window = Some(window.clone());
        let proxy = self.proxy.clone();
        self.platform = platform::init(
            &window,
            Arc::new(move |ev| {
                let _ = proxy.send_event(UserEvent::Platform(ev));
            }),
        );
        self.sync_window_size();
        let size = window.inner_size();
        log::info!(
            "window: {}x{} px at scale {}",
            size.width,
            size.height,
            window.scale_factor()
        );
        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        log::debug!("user event (window: {})", self.window.is_some());
        match event {
            UserEvent::Wake => self.viewer.dirty = true,
            UserEvent::Platform(ev) => self.handle_platform(ev),
        }
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.viewer.request_close();
                self.redraw();
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(w) = &self.window {
                    let level = if self.diag.is_some() {
                        log::Level::Info
                    } else {
                        log::Level::Debug
                    };
                    log::log!(
                        level,
                        "window event: {}; now {}x{} at scale {}",
                        match &event {
                            WindowEvent::Resized(s) =>
                                format!("resized to {}x{}", s.width, s.height),
                            WindowEvent::ScaleFactorChanged { scale_factor, .. } =>
                                format!("scale factor {scale_factor}"),
                            _ => String::new(),
                        },
                        w.inner_size().width,
                        w.inner_size().height,
                        w.scale_factor()
                    );
                }
                self.sync_window_size();
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if let Some(key) = Key::from_winit(&event.logical_key, self.mods) {
                        self.viewer.on_key(key);
                        self.redraw();
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.viewer
                    .on_cursor_moved([position.x as f32, position.y as f32]);
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.viewer.on_cursor_left();
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => Some(Button::Left),
                    MouseButton::Middle => Some(Button::Middle),
                    MouseButton::Right => Some(Button::Right),
                    _ => None,
                };
                if let Some(b) = b {
                    self.viewer
                        .on_mouse_button(b, state == ElementState::Pressed);
                    self.redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let ctrl = self.mods.control_key();
                match delta {
                    MouseScrollDelta::LineDelta(x, y) => {
                        self.viewer.on_wheel(Wheel::Lines(x, y), ctrl)
                    }
                    MouseScrollDelta::PixelDelta(PhysicalPosition { x, y }) => self
                        .viewer
                        .on_wheel(Wheel::Pixels(x as f32, y as f32), ctrl),
                }
                self.redraw();
            }
            WindowEvent::PinchGesture { delta, .. } => {
                self.viewer.on_pinch(delta as f32, None);
                self.redraw();
            }
            WindowEvent::Touch(t) => {
                // Pens on Windows / macOS (and fingers anywhere) arrive here, with pressure.
                let pos = [t.location.x as f32, t.location.y as f32];
                let pressure = touch_pressure(&t.force);
                match t.phase {
                    TouchPhase::Started => {
                        if self.touch_id.is_none() {
                            self.touch_id = Some(t.id);
                            self.viewer.mouse.pos = pos;
                            self.viewer.mouse.inside = true;
                            if self.viewer.mode == UiMode::Draw {
                                let erase = self.viewer.tool == crate::viewer::Tool::Eraser;
                                self.viewer.pen_down(pos, pressure, erase);
                            } else {
                                self.viewer.on_mouse_button(Button::Left, true);
                            }
                        }
                    }
                    TouchPhase::Moved if self.touch_id == Some(t.id) => {
                        if self.viewer.pen.is_active() {
                            self.viewer.mouse.pos = pos;
                            self.viewer.pen_move(pos, pressure);
                        } else {
                            self.viewer.on_cursor_moved(pos);
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled if self.touch_id == Some(t.id) => {
                        self.touch_id = None;
                        if self.viewer.pen.is_active() {
                            self.viewer.pen_up();
                        } else {
                            self.viewer.on_mouse_button(Button::Left, false);
                        }
                    }
                    _ => {}
                }
                self.viewer.dirty = true;
                self.redraw();
            }
            WindowEvent::DroppedFile(path) => {
                if self.viewer.is_dirty() {
                    self.viewer
                        .error("E37: No write since last change (add ! to override)");
                } else {
                    self.viewer.open(path, LoadPurpose::Open { page: None });
                }
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.draw_frame(event_loop),
            _ => {}
        }
        if self.viewer.quit {
            event_loop.exit();
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        // A timer we asked for (zoom debounce, message expiry, frame retry)
        // fired: nothing else will wake the drawing code.
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            self.redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(p) = &mut self.platform {
            p.pump();
        }
        if self.script.is_some() {
            self.script_step(event_loop);
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(15),
            ));
        }
        if self.viewer.dirty {
            self.redraw();
        }
        if self.viewer.quit {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.viewer.save_session();
    }
}

impl App {
    fn draw_frame(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        let now = Instant::now();
        let v = &mut self.viewer;
        v.dirty = false;
        v.poll();
        if v.cache_epoch != self.cache_epoch {
            renderer.clear_cache();
            self.cache_epoch = v.cache_epoch;
        }
        let more = v.upload_staged(renderer, 8);
        let tick = v.tick(now);
        let zoom_wake = v.schedule_tiles(renderer, now);
        v.refresh_highlights();

        if self.viewer.show_stats {
            let (avg, max) = self.stats.summary();
            let (used, cap) = renderer.cache_usage();
            self.viewer.stats_text = format!("{avg:.1}/{max:.1}ms tiles {used}/{cap}");
        }
        let ui = self.viewer.ui_state();
        let v = &self.viewer;
        let Some(d) = &v.doc else {
            // No document: still draw the (empty) window with the status line.
            let empty = crate::ink::Store::new(0);
            let layout = crate::view::Layout::default();
            let t0 = Instant::now();
            if let Some(d) = &mut self.diag {
                d.before(renderer, now);
            }
            let fs = renderer.render(&FrameInput {
                camera: &v.camera,
                layout: &layout,
                tile_scale: v.camera.scale().max(0.01),
                theme: &v.theme,
                ink: &empty,
                live: None,
                highlights: &[],
                cursor: None,
                ui: &ui,
                current_page: 0,
            });
            if let Some(d) = &mut self.diag {
                d.after(renderer, &fs, now);
            }
            self.stats.push(now, t0.elapsed().as_secs_f32() * 1000.0);
            let retry = self.retry_after(&fs, now);
            self.finish_frame(
                event_loop,
                &window,
                more,
                tick.animating,
                [tick.wake, retry].into_iter().flatten().min(),
                zoom_wake,
            );
            return;
        };
        let input = FrameInput {
            camera: &v.camera,
            layout: &d.layout,
            tile_scale: d.tile_scale,
            theme: &v.theme,
            ink: &d.ink,
            live: v.live_stroke(),
            highlights: &v.highlights,
            cursor: v.pen_cursor(),
            ui: &ui,
            current_page: v.current_page(),
        };
        let t0 = Instant::now();
        if let Some(d) = &mut self.diag {
            d.before(renderer, now);
        }
        let fs = renderer.render(&input);
        if let Some(d) = &mut self.diag {
            d.after(renderer, &fs, now);
        }
        if !self.presented_once && fs.skipped.is_none() {
            self.presented_once = true;
            let [w, h] = renderer.surface_size();
            log::info!(
                "first frame presented: {w}x{h} px, scale {}, {} page(s)",
                v.camera.dpr,
                d.layout.pages.len()
            );
        }
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        self.stats.push(now, ms);
        log::trace!("frame {ms:.2}ms {fs:?}");
        let retry = self.retry_after(&fs, now);
        self.finish_frame(
            event_loop,
            &window,
            more,
            tick.animating,
            [tick.wake, retry].into_iter().flatten().min(),
            zoom_wake,
        );
    }

    /// A frame that could not be drawn (or left the swapchain stale) has to
    /// be tried again, otherwise the window stays blank until the next input.
    fn retry_after(&mut self, fs: &crate::render::FrameStats, now: Instant) -> Option<Instant> {
        if fs.redraw_soon {
            self.viewer.dirty = true;
        }
        match fs.skipped {
            Some("outdated") | Some("lost") => {
                self.viewer.dirty = true;
                None
            }
            Some(_) => Some(now + Duration::from_millis(30)),
            None => None,
        }
    }

    fn finish_frame(
        &mut self,
        event_loop: &ActiveEventLoop,
        window: &Arc<Window>,
        more_uploads: bool,
        animating: bool,
        wake: Option<Instant>,
        zoom_wake: Option<Instant>,
    ) {
        let title = self.viewer.title();
        if title != self.title {
            window.set_title(&title);
            self.title = title;
        }
        let icon = if self.viewer.mode == UiMode::Draw {
            CursorIcon::Crosshair
        } else if self.viewer.mouse.panning || self.viewer.mouse.middle {
            CursorIcon::Grabbing
        } else if self.viewer.hover_link {
            CursorIcon::Pointer
        } else {
            CursorIcon::Default
        };
        window.set_cursor(icon);

        if more_uploads || animating || self.viewer.dirty {
            window.request_redraw();
        }
        // The test driver has to keep polling even when nothing else is due.
        let poll = self
            .script
            .is_some()
            .then(|| Instant::now() + Duration::from_millis(15));
        let next = [wake, zoom_wake, poll].into_iter().flatten().min();
        event_loop.set_control_flow(match next {
            Some(t) => ControlFlow::WaitUntil(t),
            None => ControlFlow::Wait,
        });
        if self.viewer.quit {
            event_loop.exit();
        }
    }
}
