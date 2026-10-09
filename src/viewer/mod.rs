//! All viewer state and behaviour, independent of the window system.
//!
//! The window glue (`app.rs`) feeds events in and reads back what to draw.

mod actions;
mod draw;
pub mod input;
mod search;
mod status;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crossbeam_channel::{unbounded, Receiver};

use crate::config::Settings;
use crate::doc::service::{Job, LinkInfo, Service};
use crate::doc::worker::{Pool, Rendered, ThumbPixels, TileKey, TilePixels};
use crate::doc::{self, DocInfo, OpenError, OutlineItem, PageMeta};
use crate::ink::{History, Store};
use crate::input::{KeyEngine, Keymaps};
use crate::render::tiles;
use crate::render::ui::Ui;
use crate::render::{Highlight, Theme};
use crate::session::{FileState, Session};
use crate::view::spring;
use crate::view::{Camera, Layout, ZoomMode};
use crate::watch::Watcher;

pub use search::Search;

/// Logical pixels per point at "100 %" (96 dpi).
pub const PT_TO_PX: f32 = 96.0 / 72.0;
const ZOOM_DEBOUNCE: Duration = Duration::from_millis(120);
const MESSAGE_TIME: Duration = Duration::from_secs(4);
const CLOSE_ARM_TIME: Duration = Duration::from_secs(3);
const MAX_JUMPS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiMode {
    Normal,
    Draw,
    Command,
    Search { forward: bool },
    Outline,
    Password,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tool {
    Pen,
    Eraser,
}

#[derive(Clone, Debug)]
pub struct Msg {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

/// A place in the document: page and y offset inside it (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pos {
    pub page: usize,
    pub y: f32,
}

struct Saving {
    id: u64,
    revision: u64,
    quit_after: bool,
}

pub struct DocState {
    pub path: PathBuf,
    pub password: Option<String>,
    pub layout: Layout,
    pub metas: Vec<PageMeta>,
    pub ink: Store,
    pub history: History,
    pub pool: Pool,
    pub service: Service,
    pub outline: Vec<OutlineItem>,
    pub marks: BTreeMap<char, Pos>,
    jumps: Vec<Pos>,
    jump_idx: usize,
    pub links: HashMap<usize, Vec<LinkInfo>>,
    links_requested: HashSet<usize>,
    pub search: Search,
    pub mtime: Option<SystemTime>,
    saving: Option<Saving>,
    save_seq: u64,
    _watcher: Option<Watcher>,
    watcher_rx: Option<Receiver<()>>,
    pub tile_scale: f32,
}

pub enum LoadPurpose {
    /// First open of a file in this window.
    Open { page: Option<usize> },
    /// Reload the same file, keeping the view.
    Reload,
}

struct Loader {
    rx: Receiver<Result<DocInfo, OpenError>>,
    purpose: LoadPurpose,
    path: PathBuf,
}

#[derive(Default)]
pub struct MouseState {
    pub pos: [f32; 2],
    pub left: bool,
    pub middle: bool,
    pub right: bool,
    /// Where a left-button press started, for click vs. drag.
    pub press_at: Option<[f32; 2]>,
    pub panning: bool,
    pub inside: bool,
}

pub struct Viewer {
    pub settings: Settings,
    pub keymaps: Keymaps,
    engine: KeyEngine,
    pub session: Session,
    pub doc: Option<DocState>,
    loader: Option<Loader>,
    pub mode: UiMode,
    pub dark: bool,
    pub camera: Camera,
    pub window_size: [u32; 2],
    pub message: Option<Msg>,
    /// Text of the `:` / `/` / password line.
    pub line: String,
    history_cmd: Vec<String>,
    history_idx: Option<usize>,
    outline_filter: String,
    outline_filtering: bool,
    outline_sel: usize,
    password_attempts: u8,
    password_path: Option<(PathBuf, LoadPurpose)>,
    pub tool: Tool,
    pub pen_color: [u8; 3],
    pub pen_width: f32,
    pub pen: draw::PenState,
    pub mouse: MouseState,
    pub quit: bool,
    close_armed: Option<Instant>,
    zoom_changed_at: Option<Instant>,
    last_scale: f32,
    anim_target: Option<[f32; 2]>,
    /// Velocity of the scroll spring (document units per second).
    anim_vel: [f32; 2],
    /// The previous tick advanced the animation (so `last_tick` is a real
    /// frame interval).
    anim_running: bool,
    last_tick: Instant,
    /// Anything changed that needs a redraw.
    pub dirty: bool,
    /// Bumped when the renderer's cache must be dropped (reload).
    pub cache_epoch: u64,
    staged_tiles: VecDeque<TilePixels>,
    staged_keys: HashSet<TileKey>,
    staged_thumbs: VecDeque<ThumbPixels>,
    wake: Arc<dyn Fn() + Send + Sync>,
    pub hover_link: bool,
    pub highlights: Vec<Highlight>,
    pub theme: Theme,
    pub show_stats: bool,
    pub stats_text: String,
    fired: Vec<crate::input::keymap::Fired>,
}

impl Viewer {
    pub fn new(
        settings: Settings,
        config_warning: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Viewer {
        let mut keymaps = Keymaps::defaults();
        let mut warnings = keymaps.apply_overrides(&settings.keys_normal, &settings.keys_draw);
        if let Some(w) = config_warning {
            warnings.insert(0, w);
        }
        let theme = Theme {
            dark: settings.dark_by_default,
            dark_bg: settings.dark_bg,
            dark_fg: settings.dark_fg,
            separator: settings.dark_separator,
        };
        let mut v = Viewer {
            pen_color: settings
                .palette
                .first()
                .copied()
                .unwrap_or([0x1a, 0x1a, 0x1a]),
            pen_width: settings.pen_width,
            dark: settings.dark_by_default,
            theme,
            keymaps,
            engine: KeyEngine::default(),
            session: Session::load(),
            doc: None,
            loader: None,
            mode: UiMode::Normal,
            camera: Camera::default(),
            window_size: [800, 600],
            message: None,
            line: String::new(),
            history_cmd: Vec::new(),
            history_idx: None,
            outline_filter: String::new(),
            outline_filtering: false,
            outline_sel: 0,
            password_attempts: 0,
            password_path: None,
            tool: Tool::Pen,
            pen: draw::PenState::default(),
            mouse: MouseState::default(),
            quit: false,
            close_armed: None,
            zoom_changed_at: None,
            last_scale: 0.0,
            anim_target: None,
            anim_vel: [0.0; 2],
            anim_running: false,
            last_tick: Instant::now(),
            dirty: true,
            cache_epoch: 0,
            staged_tiles: VecDeque::new(),
            staged_keys: HashSet::new(),
            staged_thumbs: VecDeque::new(),
            wake,
            hover_link: false,
            highlights: Vec::new(),
            show_stats: false,
            stats_text: String::new(),
            fired: Vec::new(),
            settings,
        };
        if !warnings.is_empty() {
            v.error(warnings.join("; "));
        }
        v
    }

    // ------------------------------------------------------------------
    // messages
    // ------------------------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Msg {
            text: text.into(),
            error: false,
            until: Instant::now() + MESSAGE_TIME,
        });
        self.dirty = true;
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Msg {
            text: text.into(),
            error: true,
            until: Instant::now() + MESSAGE_TIME,
        });
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // window
    // ------------------------------------------------------------------

    /// Tell the viewer how large the window is and how sharp the display is.
    pub fn set_window(&mut self, size: [u32; 2], dpr: f32) {
        self.window_size = size;
        self.camera.dpr = dpr;
        let bar = if self.settings.statusbar {
            Ui::bar_height_for(dpr)
        } else {
            0.0
        };
        self.camera.viewport = [size[0].max(1) as f32, (size[1] as f32 - bar).max(1.0)];
        self.relayout();
    }

    /// Re-apply fit modes and clamping after any size / layout change.
    fn relayout(&mut self) {
        if let Some(d) = &self.doc {
            let page = d.layout.page_at_y(self.camera.center_y());
            // Keep the page at the top of the viewport while the zoom changes.
            let top = self.camera.offset[1];
            self.camera.apply_mode(&d.layout, page);
            self.camera.offset[1] = top;
            self.camera.clamp(&d.layout);
        }
        self.dirty = true;
    }

    pub fn title(&self) -> String {
        match &self.doc {
            Some(d) => format!(
                "{} — mizu",
                d.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            None => "mizu".to_string(),
        }
    }

    // ------------------------------------------------------------------
    // opening and reloading
    // ------------------------------------------------------------------

    pub fn open(&mut self, path: PathBuf, purpose: LoadPurpose) {
        self.start_load(path, None, purpose);
    }

    fn start_load(&mut self, path: PathBuf, password: Option<String>, purpose: LoadPurpose) {
        let (tx, rx) = unbounded();
        let wake = self.wake.clone();
        let p = path.clone();
        let spawned = std::thread::Builder::new()
            .name("mizu-load".into())
            .spawn(move || {
                let _ = tx.send(doc::load(&p, password.as_deref()));
                wake();
            });
        if spawned.is_err() {
            self.error("cannot start loader thread");
            return;
        }
        self.loader = Some(Loader { rx, purpose, path });
        self.dirty = true;
    }

    pub fn is_loading(&self) -> bool {
        self.loader.is_some()
    }

    fn poll_loader(&mut self) {
        let Some(l) = &self.loader else { return };
        let Ok(result) = l.rx.try_recv() else { return };
        log::debug!("loader finished: ok={}", result.is_ok());
        let Loader { purpose, path, .. } = self.loader.take().expect("checked above");
        match result {
            Ok(info) => self.apply_loaded(info, purpose),
            Err(OpenError::NeedsPassword) => {
                self.password_attempts = 0;
                self.password_path = Some((path, purpose));
                self.line.clear();
                self.mode = UiMode::Password;
                self.dirty = true;
            }
            Err(OpenError::WrongPassword) => {
                self.password_attempts += 1;
                if self.password_attempts >= 3 {
                    self.password_path = None;
                    self.mode = UiMode::Normal;
                    self.error("Wrong password");
                } else {
                    self.password_path = Some((path, purpose));
                    self.line.clear();
                    self.mode = UiMode::Password;
                    self.error("Wrong password, try again");
                }
            }
            Err(OpenError::Failed(e)) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.error(format!("Cannot open \"{name}\": {e}"));
            }
        }
    }

    fn apply_loaded(&mut self, info: DocInfo, purpose: LoadPurpose) {
        let sizes: Vec<(f32, f32)> = info.pages.iter().map(|p| (p.w, p.h)).collect();
        let layout = Layout::new(&sizes);
        let n = layout.len();
        let mut ink = Store::new(n);
        for s in info.strokes.iter().cloned() {
            ink.push(s);
        }
        let wake = self.wake.clone();
        let pool = Pool::spawn(
            info.path.clone(),
            info.password.clone(),
            wake.clone(),
            crate::doc::worker::worker_count(),
        );
        let service = Service::spawn(info.path.clone(), info.password.clone(), wake.clone());
        let watcher = crate::watch::watch(&info.path, wake);
        let watcher_rx = watcher.as_ref().map(|w| w.rx.clone());

        // Carry navigation state across a reload.
        let (marks, jumps, jump_idx, keep_view) = match (&purpose, self.doc.take()) {
            (LoadPurpose::Reload, Some(old)) => {
                let cap = |p: Pos| Pos {
                    page: p.page.min(n.saturating_sub(1)),
                    y: p.y,
                };
                (
                    old.marks.iter().map(|(k, v)| (*k, cap(*v))).collect(),
                    old.jumps.iter().map(|p| cap(*p)).collect(),
                    old.jump_idx.min(old.jumps.len()),
                    true,
                )
            }
            (_, old) => {
                // Opening something else: remember where we were in the old file.
                if let Some(old) = old {
                    self.remember_view_of(&old);
                }
                (BTreeMap::new(), Vec::new(), 0, false)
            }
        };

        let saved = self.session.get(&info.path).cloned();
        let mut state = DocState {
            path: info.path.clone(),
            password: info.password.clone(),
            layout,
            metas: info.pages.clone(),
            ink,
            history: History::default(),
            pool,
            service,
            outline: info.outline.clone(),
            marks,
            jump_idx: jump_idx.min(jumps.len()),
            jumps,
            links: HashMap::new(),
            links_requested: HashSet::new(),
            search: Search::default(),
            mtime: info.mtime,
            saving: None,
            save_seq: 0,
            _watcher: watcher,
            watcher_rx,
            tile_scale: 1.0,
        };

        if !keep_view {
            let mut page_override = None;
            if let LoadPurpose::Open { page } = &purpose {
                page_override = *page;
            }
            match &saved {
                Some(s) => {
                    self.camera.mode = match s.mode.as_str() {
                        "fit_page" => ZoomMode::FitPage,
                        "free" => ZoomMode::Free,
                        _ => ZoomMode::FitWidth,
                    };
                    if self.camera.mode == ZoomMode::Free && s.zoom > 0.0 {
                        self.camera.zoom = s.zoom;
                    }
                    if let Some(d) = s.dark {
                        self.set_dark(d);
                    }
                    for (c, (page, y)) in &s.marks {
                        state.marks.insert(*c, Pos { page: *page, y: *y });
                    }
                }
                None => {
                    self.camera.mode = ZoomMode::FitWidth;
                    self.camera.offset = [0.0, 0.0];
                    self.set_dark(self.settings.dark_by_default);
                }
            }
            self.camera.apply_mode(&state.layout, 0);
            self.camera.offset = [0.0, 0.0];
            if let Some(s) = &saved {
                if s.page < n {
                    self.camera.offset = [s.x, state.layout.pages[s.page].y + s.y_in_page];
                }
            }
            if let Some(p) = page_override {
                let p = p.min(n.saturating_sub(1));
                self.camera.offset[1] = state.layout.pages[p].y;
            }
        } else {
            let page = state.layout.page_at_y(self.camera.center_y());
            self.camera.apply_mode(&state.layout, page);
        }
        self.camera.clamp(&state.layout);
        state.tile_scale = self.camera.scale();
        self.last_scale = self.camera.scale();
        self.zoom_changed_at = None;
        self.anim_target = None;
        self.staged_tiles.clear();
        self.staged_keys.clear();
        self.staged_thumbs.clear();
        self.cache_epoch += 1;
        self.pen = draw::PenState::default();
        if matches!(
            self.mode,
            UiMode::Password | UiMode::Outline | UiMode::Command | UiMode::Search { .. }
        ) {
            self.mode = UiMode::Normal;
        }
        self.doc = Some(state);
        if keep_view {
            self.info("reloaded");
        }
        self.dirty = true;
    }

    fn remember_view_of(&mut self, d: &DocState) {
        let page = d.layout.page_at_y(self.camera.offset[1]);
        let g = d.layout.pages[page];
        let st = FileState {
            page,
            y_in_page: self.camera.offset[1] - g.y,
            x: self.camera.offset[0],
            zoom: self.camera.zoom,
            mode: match self.camera.mode {
                ZoomMode::FitWidth => "fit_width",
                ZoomMode::FitPage => "fit_page",
                ZoomMode::Free => "free",
            }
            .into(),
            dark: Some(self.dark),
            marks: d.marks.iter().map(|(c, p)| (*c, (p.page, p.y))).collect(),
            stamp: 0,
        };
        self.session.set(&d.path, st);
    }

    /// Persist the session (call when quitting or switching files).
    pub fn save_session(&mut self) {
        if let Some(d) = self.doc.take() {
            self.remember_view_of(&d);
            self.doc = Some(d);
        }
        if let Err(e) = self.session.save() {
            log::warn!("cannot save session: {e}");
        }
    }

    pub fn set_dark(&mut self, on: bool) {
        self.dark = on;
        self.theme.dark = on;
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // per-frame work
    // ------------------------------------------------------------------

    /// Process everything that arrived from background threads.
    pub fn poll(&mut self) {
        self.poll_loader();
        self.poll_workers();
        self.poll_service();
        self.poll_watcher();
    }

    fn poll_workers(&mut self) {
        let Some(d) = &self.doc else { return };
        let mut err: Option<String> = None;
        let mut got = false;
        while let Ok(r) = d.pool.rx.try_recv() {
            got = true;
            match r {
                Rendered::Tile(t) => {
                    self.staged_keys.insert(t.key);
                    self.staged_tiles.push_back(t);
                }
                Rendered::Thumb(t) => self.staged_thumbs.push_back(t),
                Rendered::Failed { page, error } => {
                    err = Some(format!("page {}: {error}", page + 1))
                }
                Rendered::OpenFailed(e) => err = Some(format!("render worker: {e}")),
            }
        }
        if got {
            self.dirty = true;
        }
        if let Some(e) = err {
            self.error(e);
        }
    }

    /// Hand staged pixels to the renderer: at most `max_tiles` tiles and a
    /// few previews per frame so one frame never stalls on uploads.
    /// Returns true if more are waiting.
    pub fn upload_staged(
        &mut self,
        renderer: &mut crate::render::Renderer,
        max_tiles: usize,
    ) -> bool {
        let current = self.current_page();
        let mut n = 0;
        while n < max_tiles {
            let Some(t) = self.staged_tiles.pop_front() else {
                break;
            };
            self.staged_keys.remove(&t.key);
            if renderer.insert_tile(&t) {
                n += 1;
            }
            if let Some(d) = &self.doc {
                d.pool.recycle(t.data);
            }
        }
        let mut th = 0;
        while th < 4 {
            let Some(t) = self.staged_thumbs.pop_front() else {
                break;
            };
            renderer.insert_thumb(&t, current);
            if let Some(d) = &self.doc {
                d.pool.recycle(t.data);
            }
            th += 1;
        }
        !self.staged_tiles.is_empty() || !self.staged_thumbs.is_empty()
    }

    /// Ask the workers for what the current view needs. Returns the time at
    /// which this should be called again (zoom debounce), if any.
    pub fn schedule_tiles(
        &mut self,
        renderer: &crate::render::Renderer,
        now: Instant,
    ) -> Option<Instant> {
        let cs = self.camera.scale();
        let cam = self.camera;
        let staged = &self.staged_keys;
        let Some(d) = &mut self.doc else { return None };

        if (cs - self.last_scale).abs() > 1e-4 * cs.max(1.0) {
            self.last_scale = cs;
            self.zoom_changed_at = Some(now);
            d.pool.clear_wanted();
        }
        if let Some(t) = self.zoom_changed_at {
            let due = t + ZOOM_DEBOUNCE;
            if now < due {
                return Some(due);
            }
            self.zoom_changed_at = None;
            d.tile_scale = cs;
        }
        let [_, y0, _, y1] = cam.visible_doc_rect();
        let vis = d.layout.visible(y0, y1);
        let current = d.layout.page_at_y(cam.center_y());
        let ts = d.tile_scale;
        let wanted = tiles::wanted_tiles(&cam, &d.layout.pages, vis, ts, |k| {
            renderer.has_tile(k) || staged.contains(k)
        });
        let thumbs =
            tiles::wanted_thumbs(current, d.layout.len(), 40, 24, |p| renderer.has_thumb(p));
        d.pool.set_wanted(wanted, thumbs);

        // Links for pages we can see.
        let [_, y0, _, y1] = cam.visible_doc_rect();
        let vis = d.layout.visible(y0, y1);
        let need: Vec<usize> = vis.filter(|p| d.links_requested.insert(*p)).collect();
        if !need.is_empty() {
            d.service.send(Job::Links { pages: need });
        }
        None
    }

    pub fn is_animating(&self) -> bool {
        self.anim_target.is_some()
    }

    /// True when everything visible is sharp and nothing is moving.
    pub fn view_complete(&self, renderer: &crate::render::Renderer) -> bool {
        if self.is_loading() || self.anim_target.is_some() || self.zoom_changed_at.is_some() {
            return false;
        }
        if !self.staged_tiles.is_empty() || !self.staged_thumbs.is_empty() {
            return false;
        }
        let Some(d) = &self.doc else { return true };
        let [_, y0, _, y1] = self.camera.visible_doc_rect();
        let vis = d.layout.visible(y0, y1);
        let want = tiles::wanted_tiles(&self.camera, &d.layout.pages, vis, d.tile_scale, |k| {
            renderer.has_tile(k)
        });
        // Prefetch tiles are included in `wanted`; only the visible ones matter here.
        let cam = &self.camera;
        want.iter().all(|k| {
            let g = &d.layout.pages[k.page as usize];
            let origin = tiles::page_origin(cam, g);
            let ratio = cam.scale() / d.tile_scale;
            let ppx = tiles::page_px(g, d.tile_scale);
            match tiles::tile_range(origin, ratio, ppx, cam.viewport, 0.0) {
                Some(r) => {
                    !(k.tx as u32 >= r.tx0
                        && k.tx as u32 <= r.tx1
                        && k.ty as u32 >= r.ty0
                        && k.ty as u32 <= r.ty1)
                }
                None => true,
            }
        })
    }

    /// Advance animations and timers. Returns when to wake up next and whether
    /// an animation is running (then a redraw is needed right away).
    pub fn tick(&mut self, now: Instant) -> Tick {
        let dt = now.saturating_duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;
        let mut animating = false;
        let mut wake: Option<Instant> = None;
        let bump = |t: Instant, wake: &mut Option<Instant>| {
            *wake = Some(wake.map_or(t, |w| w.min(t)));
        };

        if let Some(target) = self.anim_target {
            // After idle, the time since the last frame is meaningless.
            let dt = if self.anim_running {
                dt.min(spring::MAX_DT)
            } else {
                spring::FIRST_DT
            };
            let s = self.camera.scale();
            let mut done = true;
            let axes = self.camera.offset.iter_mut().zip(&mut self.anim_vel);
            for ((off, vel), tgt) in axes.zip(target) {
                let (p, v) = spring::step(*off, *vel, tgt, dt, spring::OMEGA);
                // Within a third of a pixel and nearly still: snap.
                if (tgt - p).abs() * s > 0.3 || v.abs() * s > 30.0 {
                    *off = p;
                    *vel = v;
                    done = false;
                } else {
                    *off = tgt;
                    *vel = 0.0;
                }
            }
            if done {
                self.anim_target = None;
            } else {
                animating = true;
            }
            self.dirty = true;
        } else {
            self.anim_vel = [0.0; 2];
        }
        self.anim_running = animating;
        if let Some(m) = &self.message {
            if now >= m.until {
                self.message = None;
                self.dirty = true;
            } else {
                bump(m.until, &mut wake);
            }
        }
        if let Some(t) = self.close_armed {
            if now >= t + CLOSE_ARM_TIME {
                self.close_armed = None;
            }
        }
        if let Some(t) = self.zoom_changed_at {
            bump(t + ZOOM_DEBOUNCE, &mut wake);
        }
        Tick { animating, wake }
    }

    // ------------------------------------------------------------------
    // positions
    // ------------------------------------------------------------------

    pub fn page_count(&self) -> usize {
        self.doc.as_ref().map(|d| d.layout.len()).unwrap_or(0)
    }

    /// The page crossing the vertical centre of the view.
    pub fn current_page(&self) -> usize {
        self.doc
            .as_ref()
            .map(|d| d.layout.page_at_y(self.camera.center_y()))
            .unwrap_or(0)
    }

    /// Top of the viewport as a page position.
    pub fn top_pos(&self) -> Option<Pos> {
        let d = self.doc.as_ref()?;
        let page = d.layout.page_at_y(self.camera.offset[1]);
        Some(Pos {
            page,
            y: self.camera.offset[1] - d.layout.pages[page].y,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.doc.as_ref().map(|d| d.path.as_path())
    }

    pub fn is_dirty(&self) -> bool {
        self.doc
            .as_ref()
            .map(|d| d.history.is_dirty())
            .unwrap_or(false)
    }

    pub fn zoom_percent(&self) -> f32 {
        self.camera.zoom / PT_TO_PX * 100.0
    }

    /// Page under a screen position and the point inside it (page space).
    /// With `clamp`, points outside the page are pulled onto its edge.
    pub fn locate(&self, screen: [f32; 2], clamp: bool) -> Option<(usize, [f32; 2])> {
        let d = self.doc.as_ref()?;
        let p = self.camera.screen_to_doc(screen);
        let page = d.layout.page_at_y(p[1]);
        let g = d.layout.pages.get(page)?;
        let (x, y) = (p[0] - g.x, p[1] - g.y);
        if clamp {
            Some((page, [x.clamp(0.0, g.w), y.clamp(0.0, g.h)]))
        } else if x >= 0.0 && y >= 0.0 && x <= g.w && y <= g.h {
            Some((page, [x, y]))
        } else {
            None
        }
    }

    pub fn camera_moved(&mut self) {
        self.dirty = true;
    }
}

pub struct Tick {
    pub animating: bool,
    pub wake: Option<Instant>,
}
