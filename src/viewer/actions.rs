//! What the keys and `:` commands actually do.

use std::path::PathBuf;
use std::time::Instant;

use super::{ListEntry, ListKind, LoadPurpose, Pos, Saving, Tool, UiMode, Viewer, MAX_JUMPS};
use crate::doc::service::Job;
use crate::input::command::{self, Command};
use crate::input::{Action, Mode};
use crate::view::ZoomMode;

const E37: &str = "E37: No write since last change (add ! to override)";

impl Viewer {
    pub fn exec(&mut self, action: Action, count: Option<u32>) {
        use Action::*;
        let n = count.unwrap_or(1).max(1) as f32;
        let dpr = self.camera.dpr;
        let step = self.settings.scroll_step * dpr;
        let view_h = self.camera.viewport[1];
        match action {
            None => {}
            ScrollDown => self.scroll_by(0.0, step * n, true),
            ScrollUp => self.scroll_by(0.0, -step * n, true),
            ScrollLeft => self.scroll_by(-step * n, 0.0, true),
            ScrollRight => self.scroll_by(step * n, 0.0, true),
            HalfPageDown => self.scroll_by(0.0, view_h * 0.5 * n, true),
            HalfPageUp => self.scroll_by(0.0, -view_h * 0.5 * n, true),
            PageDown => self.scroll_by(0.0, (view_h - step * 0.5) * n, true),
            PageUp => self.scroll_by(0.0, -(view_h - step * 0.5) * n, true),
            NextPage => {
                let p = self.current_page() + n as usize;
                self.goto_page(p, true);
            }
            PrevPage => {
                // First `K` goes to the top of the current page if we are below it.
                let cur = self.current_page();
                let at_top = self
                    .top_pos()
                    .map(|t| t.page == cur && t.y.abs() < 2.0)
                    .unwrap_or(true);
                let target = if at_top || n > 1.0 {
                    cur.saturating_sub(n as usize)
                } else {
                    cur
                };
                self.goto_page(target, true);
            }
            GotoFirst => {
                self.push_jump();
                match count {
                    Some(c) => self.goto_page(c as usize - 1, false),
                    Option::None => self.goto_page(0, false),
                }
            }
            GotoLast => {
                self.push_jump();
                match count {
                    Some(c) => self.goto_page(c as usize - 1, false),
                    Option::None => {
                        let last = self.page_count().saturating_sub(1);
                        self.goto_page(last, false);
                        // Show the bottom of the last page, not its top.
                        if let Some(d) = &self.doc {
                            let off = [self.camera.offset[0], d.layout.height];
                            self.camera.offset = self.camera.clamped(&d.layout, off);
                        }
                    }
                }
            }
            ZoomIn | ZoomOut => {
                let f = self.settings.zoom_step.powf(n);
                let f = if action == ZoomIn { f } else { 1.0 / f };
                let c = self.camera.viewport_center();
                self.zoom_by(f, c);
            }
            FitWidth => self.set_mode(ZoomMode::FitWidth),
            FitPage => {
                self.set_mode(ZoomMode::FitPage);
                let p = self.current_page();
                self.goto_page(p, false);
            }
            Zoom100 => {
                let c = self.camera.viewport_center();
                let z = super::PT_TO_PX;
                self.camera.set_zoom_at(z, c);
                self.after_camera_change();
            }
            SearchForward => self.begin_line(UiMode::Search { forward: true }),
            SearchBackward => self.begin_line(UiMode::Search { forward: false }),
            SearchNext => self.search_step(true),
            SearchPrev => self.search_step(false),
            SetMark(c) => {
                if c.is_ascii_alphabetic() {
                    if let (Some(pos), Some(d)) = (self.top_pos(), self.doc.as_mut()) {
                        d.marks.insert(c, pos);
                        self.info(format!("mark '{c}' set"));
                    }
                } else {
                    self.error("E191: Argument must be a letter");
                }
            }
            JumpMark(c) => {
                let pos = self.doc.as_ref().and_then(|d| d.marks.get(&c).copied());
                match pos {
                    Some(p) => {
                        self.push_jump();
                        self.goto_pos(p, false);
                    }
                    Option::None => self.error("E20: Mark not set"),
                }
            }
            JumpBack => self.jump_back(),
            JumpForward => self.jump_forward(),
            Outline => self.open_outline(),
            Help => self.open_help(),
            ToggleDark => {
                let on = !self.dark;
                self.set_dark(on);
            }
            EnterDraw => {
                if self.is_reflowable() {
                    self.error("Drawing works only in PDFs");
                } else if self.doc.is_some() {
                    self.mode = UiMode::Draw;
                    self.dirty = true;
                } else {
                    self.error("No file open");
                }
            }
            ExitDraw => {
                self.pen.cancel();
                self.mode = UiMode::Normal;
                self.dirty = true;
            }
            Undo => self.undo_redo(true),
            Redo => self.undo_redo(false),
            Reload => self.reload(false),
            CommandMode => self.begin_line(UiMode::Command),
            WriteQuit => self.cmd_exit(),
            QuitForce => self.quit_now(),
            ClearMessage => {
                self.message = Option::None;
                if let Some(d) = &mut self.doc {
                    d.search.visible = false;
                }
                self.dirty = true;
            }
            ToggleEraser => {
                self.tool = if self.tool == Tool::Eraser {
                    Tool::Pen
                } else {
                    Tool::Eraser
                };
                self.info(if self.tool == Tool::Eraser {
                    "eraser"
                } else {
                    "pen"
                });
                self.dirty = true;
            }
            PenTool => {
                self.tool = Tool::Pen;
                self.dirty = true;
            }
            SelectColor(i) => self.select_palette(i),
            WidthDown => self.set_pen_width(self.pen_width / 1.25),
            WidthUp => self.set_pen_width(self.pen_width * 1.25),
        }
        self.dirty = true;
    }

    pub fn set_pen_width(&mut self, w: f32) {
        self.pen_width = w.clamp(0.25, 20.0);
        self.dirty = true;
    }

    /// Pick palette entry `i` (1-based).
    pub fn select_palette(&mut self, i: u8) {
        match self.settings.palette.get((i as usize).wrapping_sub(1)) {
            Some(&c) => self.select_color(c, Some(i)),
            Option::None => self.error(format!(
                "No colour {i} in the palette (it has {})",
                self.settings.palette.len()
            )),
        }
    }

    pub fn select_color(&mut self, c: [u8; 3], index: Option<u8>) {
        self.pen_color = c;
        self.tool = Tool::Pen;
        let hex = command::format_hex(c);
        self.info(match index {
            Some(i) => format!("colour {i}  {hex}"),
            Option::None => format!("colour {hex}"),
        });
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // camera
    // ------------------------------------------------------------------

    pub fn after_camera_change(&mut self) {
        self.anim_target = None;
        if let Some(d) = &self.doc {
            self.camera.clamp(&d.layout);
        }
        self.dirty = true;
    }

    pub fn zoom_by(&mut self, factor: f32, anchor: [f32; 2]) {
        self.camera.zoom_by_at(factor, anchor);
        self.after_camera_change();
    }

    fn set_mode(&mut self, mode: ZoomMode) {
        self.camera.mode = mode;
        let page = self.current_page();
        if let Some(d) = &self.doc {
            self.camera.apply_mode(&d.layout, page);
        }
        self.after_camera_change();
    }

    /// Scroll by physical pixels, optionally eased.
    pub fn scroll_by(&mut self, dx: f32, dy: f32, animated: bool) {
        let Some(d) = &self.doc else { return };
        let s = self.camera.scale();
        if animated {
            let base = self.anim_target.unwrap_or(self.camera.offset);
            let target = [base[0] + dx / s, base[1] + dy / s];
            self.anim_target = Some(self.camera.clamped(&d.layout, target));
        } else {
            self.anim_target = Option::None;
            self.camera.scroll_px(dx, dy);
            self.camera.clamp(&d.layout);
        }
        self.dirty = true;
    }

    pub fn goto_page(&mut self, page: usize, animated: bool) {
        let Some(d) = &self.doc else { return };
        let page = page.min(d.layout.len().saturating_sub(1));
        let y = d.layout.pages[page].y;
        self.goto_y(y, animated);
    }

    pub fn goto_pos(&mut self, pos: Pos, animated: bool) {
        let Some(d) = &self.doc else { return };
        let page = pos.page.min(d.layout.len().saturating_sub(1));
        let y = d.layout.pages[page].y + pos.y.max(0.0);
        self.goto_y(y, animated);
    }

    fn goto_y(&mut self, y: f32, animated: bool) {
        let Some(d) = &self.doc else { return };
        // Below the title bar, where there is one.
        let y = y - self.camera.inset_doc();
        let target = self.camera.clamped(&d.layout, [self.camera.offset[0], y]);
        if animated {
            self.anim_target = Some(target);
        } else {
            self.anim_target = Option::None;
            self.camera.offset = target;
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // jump list
    // ------------------------------------------------------------------

    /// Remember where we are before a long jump.
    pub fn push_jump(&mut self) {
        let Some(pos) = self.top_pos() else { return };
        let Some(d) = &mut self.doc else { return };
        d.jumps.truncate(d.jump_idx);
        if d.jumps.last() != Some(&pos) {
            d.jumps.push(pos);
        }
        if d.jumps.len() > MAX_JUMPS {
            d.jumps.remove(0);
        }
        d.jump_idx = d.jumps.len();
    }

    fn jump_back(&mut self) {
        let Some(cur) = self.top_pos() else { return };
        let target = {
            let Some(d) = &mut self.doc else { return };
            if d.jump_idx >= d.jumps.len() {
                // At the head of the list: remember here so Ctrl-i can return.
                if d.jumps.last() != Some(&cur) {
                    d.jumps.push(cur);
                }
                d.jump_idx = d.jumps.len() - 1;
            }
            if d.jump_idx == 0 {
                None
            } else {
                d.jump_idx -= 1;
                Some(d.jumps[d.jump_idx])
            }
        };
        match target {
            Some(p) => self.goto_pos(p, false),
            Option::None => self.info("Already at the oldest jump"),
        }
    }

    fn jump_forward(&mut self) {
        let target = {
            let Some(d) = &mut self.doc else { return };
            if d.jump_idx + 1 < d.jumps.len() {
                d.jump_idx += 1;
                Some(d.jumps[d.jump_idx])
            } else {
                None
            }
        };
        match target {
            Some(p) => self.goto_pos(p, false),
            Option::None => self.info("Already at the newest jump"),
        }
    }

    // ------------------------------------------------------------------
    // undo / redo
    // ------------------------------------------------------------------

    fn undo_redo(&mut self, undo: bool) {
        let vis = {
            let [_, y0, _, y1] = self.camera.visible_doc_rect();
            self.doc.as_ref().map(|d| d.layout.visible(y0, y1))
        };
        let Some(d) = &mut self.doc else { return };
        let page = if undo {
            d.history.undo(&mut d.ink)
        } else {
            d.history.redo(&mut d.ink)
        };
        match page {
            Some(p) => {
                if !vis.map(|v| v.contains(&p)).unwrap_or(false) {
                    self.info(format!(
                        "{} on page {}",
                        if undo { "Undo" } else { "Redo" },
                        p + 1
                    ));
                }
            }
            Option::None => self.info(if undo {
                "Already at oldest change"
            } else {
                "Already at newest change"
            }),
        }
    }

    // ------------------------------------------------------------------
    // outline
    // ------------------------------------------------------------------

    fn open_outline(&mut self) {
        let Some(d) = &self.doc else { return };
        if d.outline.is_empty() {
            self.info("No outline");
            return;
        }
        let cur = self.current_page();
        let sel = d
            .outline
            .iter()
            .rposition(|o| o.page.map(|p| p <= cur).unwrap_or(false))
            .unwrap_or(0);
        self.open_list(ListKind::Outline, Vec::new());
        // Select the entry for the current position within the unfiltered list.
        self.outline_sel = sel;
    }

    /// Outline entries that match the filter, with their index in the full list.
    pub fn outline_visible(&self) -> Vec<(usize, &crate::doc::OutlineItem)> {
        let Some(d) = &self.doc else {
            return Vec::new();
        };
        let f = self.outline_filter.to_lowercase();
        d.outline
            .iter()
            .enumerate()
            .filter(|(_, o)| f.is_empty() || o.title.to_lowercase().contains(&f))
            .collect()
    }

    fn open_list(&mut self, kind: ListKind, items: Vec<ListEntry>) {
        self.list_kind = kind;
        self.list_items = items;
        self.outline_filter.clear();
        self.outline_filtering = false;
        self.outline_sel = 0;
        self.mode = UiMode::Outline;
    }

    pub fn open_help(&mut self) {
        let mut items = Vec::new();
        let section = |title: &str, items: &mut Vec<ListEntry>| {
            items.push(ListEntry {
                text: format!("── {title} ──"),
                file: Option::None,
            });
        };
        for (mode, title) in [
            (Mode::Normal, "normal mode"),
            (Mode::Draw, "drawing mode (i)"),
        ] {
            section(title, &mut items);
            for (keys, action) in self.keymaps.bindings(mode) {
                // Drawing mode inherits normal mode; list only what differs.
                if mode == Mode::Draw
                    && self
                        .keymaps
                        .bindings(Mode::Normal)
                        .contains(&(keys.clone(), action))
                {
                    continue;
                }
                items.push(ListEntry {
                    text: format!("{keys:<10} {}", action.name()),
                    file: Option::None,
                });
            }
        }
        section("commands", &mut items);
        for c in command::COMMANDS {
            let mut name = c.name.to_string();
            for a in c.aliases {
                name.push_str(", ");
                name.push_str(a);
            }
            items.push(ListEntry {
                text: format!(":{name} {}  ·  {}", c.arg_hint, c.help),
                file: Option::None,
            });
        }
        self.open_list(ListKind::Help, items);
    }

    pub fn open_recent(&mut self) {
        let items: Vec<ListEntry> = self
            .session
            .recent(100)
            .into_iter()
            .map(|(path, st)| ListEntry {
                text: format!("{}  ·  page {}", tilde(&path), st.page + 1),
                file: Some(path),
            })
            .collect();
        if items.is_empty() {
            self.info("No recent files");
            return;
        }
        self.open_list(ListKind::Recent, items);
    }

    /// Rows of the list overlay that match the filter: (index, text).
    pub fn list_visible(&self) -> Vec<(usize, String)> {
        match self.list_kind {
            ListKind::Outline => self
                .outline_visible()
                .into_iter()
                .map(|(i, o)| {
                    let indent = "  ".repeat(o.level as usize);
                    let text = match o.page {
                        Some(p) => format!("{indent}{}  ·  {}", o.title, p + 1),
                        Option::None => format!("{indent}{}", o.title),
                    };
                    (i, text)
                })
                .collect(),
            ListKind::Help | ListKind::Recent => {
                let f = self.outline_filter.to_lowercase();
                self.list_items
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| f.is_empty() || e.text.to_lowercase().contains(&f))
                    .map(|(i, e)| (i, e.text.clone()))
                    .collect()
            }
        }
    }

    /// Enter in the list overlay.
    pub fn list_activate(&mut self) {
        match self.list_kind {
            ListKind::Outline => self.outline_jump(),
            ListKind::Help => self.mode = UiMode::Normal,
            ListKind::Recent => {
                let vis = self.list_visible();
                let Some((i, _)) = vis.get(self.outline_sel.min(vis.len().saturating_sub(1)))
                else {
                    return;
                };
                let file = self.list_items[*i].file.clone();
                self.mode = UiMode::Normal;
                if let Some(p) = file {
                    if self.is_dirty() {
                        self.error(E37);
                    } else {
                        self.open(p, LoadPurpose::Open { page: Option::None });
                    }
                }
            }
        }
    }

    /// A file handed over by the system (Finder, the Dock): like `:e`.
    pub fn open_from_outside(&mut self, path: PathBuf) {
        if self.path() == Some(path.as_path()) {
            return;
        }
        if self.is_dirty() {
            self.error(E37);
            return;
        }
        self.open(path, LoadPurpose::Open { page: Option::None });
    }

    /// `:fontsize`: lay the book out again with another text size, keeping
    /// the reading position.
    pub fn set_font_size(&mut self, pt: f32) {
        if !self.is_reflowable() {
            self.error("E: :fontsize only works in EPUB books");
            return;
        }
        if !(4.0..=72.0).contains(&pt) {
            self.error("E474: Invalid argument: font size must be between 4 and 72");
            return;
        }
        self.reflow.em = pt;
        let fraction = self.reading_fraction();
        if let Some(path) = self.path().map(|p| p.to_path_buf()) {
            self.start_load(path, Option::None, LoadPurpose::Relayout { fraction });
            self.info(format!("Laying out at {pt}pt…"));
        }
    }

    pub fn outline_jump(&mut self) {
        let vis = self.outline_visible();
        let Some((_, item)) = vis.get(self.outline_sel.min(vis.len().saturating_sub(1))) else {
            return;
        };
        let (page, y) = (item.page, item.y);
        self.mode = UiMode::Normal;
        match page {
            Some(p) => {
                self.push_jump();
                self.goto_pos(
                    Pos {
                        page: p,
                        y: y.unwrap_or(0.0),
                    },
                    false,
                );
            }
            Option::None => self.error("This entry has no target"),
        }
    }

    // ------------------------------------------------------------------
    // commands
    // ------------------------------------------------------------------

    pub fn execute_command(&mut self, line: &str) {
        let cmd = match command::parse(line) {
            Ok(Some(c)) => c,
            Ok(Option::None) => return,
            Err(e) => {
                self.error(e);
                return;
            }
        };
        match cmd {
            Command::Write { path } => {
                let Some(cur) = self.path().map(|p| p.to_path_buf()) else {
                    self.error("E32: No file name");
                    return;
                };
                let dst = path.unwrap_or(cur);
                self.start_save(dst, false);
            }
            Command::Quit { force } => {
                if !force && self.is_dirty() {
                    self.error(E37);
                } else {
                    self.quit_now();
                }
            }
            Command::WriteQuit if self.is_reflowable() => self.quit_now(),
            Command::WriteQuit => match self.path().map(|p| p.to_path_buf()) {
                Some(p) => self.start_save(p, true),
                Option::None => self.quit_now(),
            },
            Command::Exit => self.cmd_exit(),
            Command::Edit { path, force } => match path {
                Some(p) => {
                    if !force && self.is_dirty() {
                        self.error(E37);
                    } else {
                        self.open(p, LoadPurpose::Open { page: Option::None });
                    }
                }
                Option::None => self.reload(force),
            },
            Command::Goto(n) => {
                if self.doc.is_none() {
                    return;
                }
                self.push_jump();
                self.goto_page(n.saturating_sub(1), false);
            }
            Command::Dark => self.set_dark(true),
            Command::Light => self.set_dark(false),
            Command::Color(c) => self.select_color(c, Option::None),
            Command::ColorIndex(i) => self.select_palette(i),
            Command::Width(Some(w)) => self.set_pen_width(w),
            Command::Width(Option::None) => self.info(format!("width {:.2}pt", self.pen_width)),
            Command::Help => self.open_help(),
            Command::Recent => self.open_recent(),
            Command::FontSize(pt) => self.set_font_size(pt),
        }
        self.dirty = true;
    }

    pub fn cmd_exit(&mut self) {
        if self.is_dirty() {
            if let Some(p) = self.path().map(|p| p.to_path_buf()) {
                self.start_save(p, true);
                return;
            }
        }
        self.quit_now();
    }

    pub fn quit_now(&mut self) {
        self.save_session();
        self.quit = true;
    }

    /// The window's close button.
    pub fn request_close(&mut self) {
        if self.is_dirty() {
            if self.close_armed.is_some() {
                self.quit_now();
            } else {
                self.close_armed = Some(Instant::now());
                self.error(E37);
            }
        } else {
            self.quit_now();
        }
    }

    pub fn reload(&mut self, force: bool) {
        let Some(path) = self.path().map(|p| p.to_path_buf()) else {
            self.error("E32: No file name");
            return;
        };
        if !force && self.is_dirty() {
            self.error("E37: No write since last change (use :e! to discard)");
            return;
        }
        self.start_load_keep(path);
    }

    fn start_load_keep(&mut self, path: PathBuf) {
        let password = self.doc.as_ref().and_then(|d| d.password.clone());
        self.start_load(path, password, LoadPurpose::Reload);
    }

    pub fn start_save(&mut self, dst: PathBuf, quit_after: bool) {
        if self.is_reflowable() {
            self.error("E: EPUB files are read-only");
            return;
        }
        let Some(d) = &mut self.doc else {
            self.error("No file open");
            return;
        };
        if d.saving.is_some() {
            self.error("A save is already in progress");
            return;
        }
        let same = dst == d.path;
        if same && !d.history.is_dirty() {
            if quit_after {
                self.quit_now();
            } else {
                self.info("No changes to write");
            }
            return;
        }
        let strokes: Vec<_> = d.ink.all().cloned().collect();
        d.save_seq += 1;
        let id = d.save_seq;
        d.saving = Some(Saving {
            id,
            revision: d.history.revision(),
            quit_after,
        });
        d.service.send(Job::Save { id, dst, strokes });
        self.info("Saving…");
    }

    /// Called when the file was replaced by an external program.
    pub(super) fn external_change(&mut self) {
        let dirty = self.is_dirty();
        let saving = self
            .doc
            .as_ref()
            .map(|d| d.saving.is_some())
            .unwrap_or(false);
        if saving {
            return;
        }
        let Some(path) = self.path().map(|p| p.to_path_buf()) else {
            return;
        };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime == self.doc.as_ref().and_then(|d| d.mtime) {
            return; // our own write
        }
        if dirty {
            self.error("W12: File changed on disk. :e! to reload (discards drawings)");
        } else {
            self.start_load_keep(path);
        }
    }
}

/// A path for display, with the home directory as `~`.
pub(super) fn tilde(p: &std::path::Path) -> String {
    if let Some(home) = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf()) {
        if let Ok(rest) = p.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    p.display().to_string()
}
