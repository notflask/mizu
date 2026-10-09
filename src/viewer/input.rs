//! Keyboard and mouse handling.

use super::{Tool, UiMode, Viewer};
use crate::doc::service::LinkTarget;
use crate::input::keymap::Mode;
use crate::input::{Key, KeyCode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug)]
pub enum Wheel {
    /// Mouse wheel notches.
    Lines(f32, f32),
    /// Touchpad, physical pixels.
    Pixels(f32, f32),
}

/// Distance in physical pixels after which a press becomes a drag.
const DRAG_THRESHOLD: f32 = 4.0;

impl Viewer {
    pub fn on_key(&mut self, key: Key) {
        self.on_key_layout(key, None);
    }

    /// `latin` is the same physical key on a US layout, used in Normal and
    /// Draw mode when a non-Latin layout produced a key nothing is bound to.
    pub fn on_key_layout(&mut self, key: Key, latin: Option<Key>) {
        match self.mode {
            UiMode::Normal | UiMode::Draw => {
                self.message = None;
                let mode = if self.mode == UiMode::Draw {
                    Mode::Draw
                } else {
                    Mode::Normal
                };
                let key = match latin {
                    Some(l) if !self.keymaps.uses_key(mode, key) => l,
                    _ => key,
                };
                self.on_key_normal(key);
            }
            UiMode::Command | UiMode::Search { .. } => self.on_key_line(key),
            UiMode::Outline => self.on_key_outline(key),
            UiMode::Password => self.on_key_password(key),
        }
        self.dirty = true;
    }

    fn on_key_normal(&mut self, key: Key) {
        let mode = if self.mode == UiMode::Draw {
            Mode::Draw
        } else {
            Mode::Normal
        };
        let mut fired = std::mem::take(&mut self.fired);
        fired.clear();
        self.engine.feed(&self.keymaps, mode, key, &mut fired);
        for f in fired.drain(..) {
            self.exec(f.action, f.count);
        }
        self.fired = fired;
    }

    fn on_key_outline(&mut self, key: Key) {
        let len = self.list_visible().len();
        let ctrl = key.mods.ctrl;
        let navigating = !self.outline_filtering && self.outline_filter.is_empty();
        let mut sel = self.outline_sel as isize;
        match key.code {
            KeyCode::Esc => {
                if self.outline_filtering || !self.outline_filter.is_empty() {
                    self.outline_filter.clear();
                    self.outline_filtering = false;
                    self.outline_sel = 0;
                } else {
                    self.mode = UiMode::Normal;
                }
                return;
            }
            KeyCode::Enter => {
                self.list_activate();
                return;
            }
            KeyCode::Up => sel -= 1,
            KeyCode::Down => sel += 1,
            KeyCode::PageUp => sel -= 10,
            KeyCode::PageDown => sel += 10,
            KeyCode::Home => sel = 0,
            KeyCode::End => sel = len as isize - 1,
            KeyCode::Char('p') | KeyCode::Char('k') if ctrl => sel -= 1,
            KeyCode::Char('n') | KeyCode::Char('j') if ctrl => sel += 1,
            KeyCode::Char('u') if ctrl => sel -= 10,
            KeyCode::Char('d') if ctrl => sel += 10,
            KeyCode::Char('k') if navigating => sel -= 1,
            KeyCode::Char('j') if navigating => sel += 1,
            KeyCode::Char('g') if navigating => sel = 0,
            KeyCode::Char('G') if navigating => sel = len as isize - 1,
            KeyCode::Char('/') if navigating => self.outline_filtering = true,
            KeyCode::Backspace => {
                self.outline_filter.pop();
                if self.outline_filter.is_empty() {
                    self.outline_filtering = false;
                }
                sel = 0;
            }
            _ => {
                if let Some(c) = key.text() {
                    self.outline_filtering = true;
                    self.outline_filter.push(c);
                    sel = 0;
                }
            }
        }
        let max = len as isize - 1;
        self.outline_sel = sel.clamp(0, max.max(0)) as usize;
    }

    fn on_key_password(&mut self, key: Key) {
        match key.code {
            KeyCode::Esc => {
                self.mode = UiMode::Normal;
                self.line.clear();
                self.password_path = None;
            }
            KeyCode::Enter => {
                let pw = std::mem::take(&mut self.line);
                self.mode = UiMode::Normal;
                if let Some((path, purpose)) = self.password_path.take() {
                    self.start_load(path, Some(pw), purpose);
                }
            }
            KeyCode::Backspace => {
                self.line.pop();
            }
            _ => {
                if let Some(c) = key.text() {
                    self.line.push(c);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // mouse
    // ------------------------------------------------------------------

    pub fn on_cursor_moved(&mut self, pos: [f32; 2]) {
        let old = self.mouse.pos;
        self.mouse.pos = pos;
        self.mouse.inside = true;
        let (dx, dy) = (pos[0] - old[0], pos[1] - old[1]);

        if self.pen.is_active() {
            self.pen_move(pos, None);
        } else if self.mouse.middle || self.mouse.panning {
            self.scroll_by(-dx, -dy, false);
        } else if self.mouse.left {
            if let Some(start) = self.mouse.press_at {
                let moved = ((pos[0] - start[0]).powi(2) + (pos[1] - start[1]).powi(2)).sqrt();
                if moved > DRAG_THRESHOLD {
                    self.mouse.panning = true;
                    self.scroll_by(-dx, -dy, false);
                }
            }
        }
        let hover = !matches!(self.mode, UiMode::Draw) && self.link_at(pos).is_some();
        if hover != self.hover_link {
            self.hover_link = hover;
        }
        self.dirty = true;
    }

    pub fn on_cursor_left(&mut self) {
        self.mouse.inside = false;
        self.dirty = true;
    }

    pub fn on_mouse_button(&mut self, button: Button, pressed: bool) {
        let pos = self.mouse.pos;
        let drawing = self.mode == UiMode::Draw && self.doc.is_some();
        match (button, pressed) {
            (Button::Left, true) => {
                self.mouse.left = true;
                self.mouse.press_at = Some(pos);
                self.mouse.panning = false;
                if drawing {
                    let erase = self.tool == Tool::Eraser;
                    self.pen_down(pos, None, erase);
                }
            }
            (Button::Left, false) => {
                self.mouse.left = false;
                let was_panning = self.mouse.panning;
                self.mouse.panning = false;
                if self.pen.is_active() {
                    self.pen_up();
                } else if !was_panning && self.mouse.press_at.is_some() && !drawing {
                    self.click(pos);
                }
                self.mouse.press_at = None;
            }
            (Button::Middle, down) => {
                self.mouse.middle = down;
            }
            (Button::Right, true) => {
                self.mouse.right = true;
                if drawing && !self.pen.is_active() {
                    self.pen_down(pos, None, true);
                }
            }
            (Button::Right, false) => {
                self.mouse.right = false;
                if self.pen.is_active()
                    && self.pen.is_erasing()
                    && (!self.mouse.left || self.tool != Tool::Eraser)
                {
                    self.pen_up();
                }
            }
        }
        self.dirty = true;
    }

    pub fn on_wheel(&mut self, wheel: Wheel, ctrl: bool) {
        let pos = self.mouse.pos;
        match wheel {
            Wheel::Lines(x, y) => {
                if ctrl {
                    self.zoom_by(1.1f32.powf(y), pos);
                } else {
                    let step = self.settings.scroll_step * self.camera.dpr;
                    self.scroll_by(-x * step, -y * step, true);
                }
            }
            Wheel::Pixels(x, y) => {
                if ctrl {
                    self.zoom_by((y * 0.006).exp(), pos);
                } else {
                    self.scroll_by(-x, -y, false);
                }
            }
        }
    }

    /// Touchpad pinch: `delta` is the relative change (0.1 = 10 % bigger).
    pub fn on_pinch(&mut self, delta: f32, pos: Option<[f32; 2]>) {
        let anchor = pos.unwrap_or(self.mouse.pos);
        self.zoom_by((1.0 + delta).max(0.05), anchor);
    }

    // ------------------------------------------------------------------
    // links
    // ------------------------------------------------------------------

    pub fn link_at(&self, screen: [f32; 2]) -> Option<&crate::doc::service::LinkInfo> {
        let (page, p) = self.locate(screen, false)?;
        let d = self.doc.as_ref()?;
        d.links.get(&page)?.iter().find(|l| {
            p[0] >= l.rect[0] && p[0] <= l.rect[2] && p[1] >= l.rect[1] && p[1] <= l.rect[3]
        })
    }

    fn click(&mut self, pos: [f32; 2]) {
        let Some(link) = self.link_at(pos).cloned() else {
            return;
        };
        match link.target {
            LinkTarget::Page { page, y } => {
                self.push_jump();
                self.goto_pos(
                    super::Pos {
                        page,
                        y: (y.unwrap_or(0.0) - 8.0).max(0.0),
                    },
                    false,
                );
            }
            LinkTarget::Uri(uri) => {
                let ok = ["http://", "https://", "mailto:"]
                    .iter()
                    .any(|s| uri.starts_with(s));
                if !ok {
                    self.error(format!("Blocked link: {uri}"));
                } else if let Err(e) = open::that_detached(&uri) {
                    self.error(format!("Cannot open {uri}: {e}"));
                } else {
                    self.info(format!("Opening {uri}"));
                }
            }
        }
    }
}
