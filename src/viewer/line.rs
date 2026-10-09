//! The `:` and `/` input line: editing with a cursor, history, suggestions
//! and Tab completion.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use super::{UiMode, Viewer};
use crate::input::command;
use crate::input::complete::{self, Suggestions};
use crate::input::{Key, KeyCode};

type DirListing = (PathBuf, Vec<(String, bool)>);

/// Suggestion state of the command line.
#[derive(Default)]
pub struct Completion {
    pub sugg: Suggestions,
    /// Row chosen with Tab / Shift-Tab, while cycling.
    pub selected: Option<usize>,
    /// Text left and right of the completed word while cycling.
    cycle: Option<(String, String)>,
    /// The listing of the last directory, valid while the line is open.
    dir_cache: RefCell<Option<DirListing>>,
}

impl Completion {
    fn reset(&mut self) {
        self.sugg = Suggestions::default();
        self.selected = None;
        self.cycle = None;
        self.dir_cache.borrow_mut().take();
    }

    fn list_dir(&self, dir: &Path) -> Option<Vec<(String, bool)>> {
        if let Some((d, entries)) = &*self.dir_cache.borrow() {
            if d == dir {
                return Some(entries.clone());
            }
        }
        let entries = command::read_dir(dir)?;
        *self.dir_cache.borrow_mut() = Some((dir.to_path_buf(), entries.clone()));
        Some(entries)
    }
}

impl Viewer {
    pub fn begin_line(&mut self, mode: UiMode) {
        self.mode = mode;
        self.line.clear();
        self.line_cursor = 0;
        self.history_idx = None;
        self.completion.reset();
        self.refresh_completion();
        self.dirty = true;
    }

    fn end_line(&mut self) {
        self.mode = UiMode::Normal;
        self.line.clear();
        self.line_cursor = 0;
        self.completion.reset();
    }

    pub(super) fn on_key_line(&mut self, key: Key) {
        let ctrl = key.mods.ctrl;
        let is_cmd = self.mode == UiMode::Command;
        let tab_key = matches!(key.code, KeyCode::Tab);
        if !tab_key {
            // Any other key keeps what Tab inserted and stops cycling.
            self.completion.cycle = None;
            self.completion.selected = None;
        }
        let before = (self.line.len(), self.line_cursor);
        let mut edited = true;
        match key.code {
            KeyCode::Esc => {
                self.end_line();
                return;
            }
            KeyCode::Char('[') if ctrl => {
                self.end_line();
                return;
            }
            KeyCode::Char('c') if ctrl => {
                self.end_line();
                return;
            }
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.line);
                let mode = self.mode;
                self.end_line();
                match mode {
                    UiMode::Command => {
                        if !line.trim().is_empty() && self.history_cmd.last() != Some(&line) {
                            self.history_cmd.push(line.clone());
                        }
                        self.execute_command(&line);
                    }
                    UiMode::Search { forward } => self.start_search(&line, forward),
                    _ => {}
                }
                return;
            }
            KeyCode::Backspace => {
                if self.line.is_empty() {
                    self.end_line();
                    return;
                }
                if let Some(p) = self.prev_boundary() {
                    self.line.replace_range(p..self.line_cursor, "");
                    self.line_cursor = p;
                }
            }
            KeyCode::Delete => {
                if let Some(n) = self.next_boundary() {
                    self.line.replace_range(self.line_cursor..n, "");
                }
            }
            KeyCode::Char('d') if ctrl => {
                if let Some(n) = self.next_boundary() {
                    self.line.replace_range(self.line_cursor..n, "");
                }
            }
            KeyCode::Char('u') if ctrl => {
                self.line.replace_range(..self.line_cursor, "");
                self.line_cursor = 0;
            }
            KeyCode::Char('k') if ctrl => self.line.truncate(self.line_cursor),
            KeyCode::Char('w') if ctrl => {
                let head = &self.line[..self.line_cursor];
                let trimmed = head.trim_end_matches([' ', '/']).len();
                let cut = head[..trimmed]
                    .rfind([' ', '/'])
                    .map(|i| i + 1)
                    .unwrap_or(0);
                self.line.replace_range(cut..self.line_cursor, "");
                self.line_cursor = cut;
            }
            KeyCode::Left => {
                edited = false;
                if let Some(p) = self.prev_boundary() {
                    self.line_cursor = p;
                }
            }
            KeyCode::Char('b') if ctrl => {
                edited = false;
                if let Some(p) = self.prev_boundary() {
                    self.line_cursor = p;
                }
            }
            KeyCode::Right | KeyCode::Char('f') if key.code == KeyCode::Right || ctrl => {
                edited = false;
                if !self.accept_ghost() {
                    if let Some(n) = self.next_boundary() {
                        self.line_cursor = n;
                    }
                }
            }
            KeyCode::Home => self.line_cursor = 0,
            KeyCode::Char('a') if ctrl => self.line_cursor = 0,
            KeyCode::End => {
                if !self.accept_ghost() {
                    self.line_cursor = self.line.len();
                }
            }
            KeyCode::Char('e') if ctrl => {
                if !self.accept_ghost() {
                    self.line_cursor = self.line.len();
                }
            }
            KeyCode::Tab if is_cmd => {
                self.tab(key.mods.shift);
                self.dirty = true;
                return;
            }
            KeyCode::Up if is_cmd => {
                edited = false;
                self.history_step(true);
            }
            KeyCode::Down if is_cmd => {
                edited = false;
                self.history_step(false);
            }
            KeyCode::Char('p') if ctrl && is_cmd => {
                edited = false;
                self.history_step(true);
            }
            KeyCode::Char('n') if ctrl && is_cmd => {
                edited = false;
                self.history_step(false);
            }
            _ => match key.text() {
                Some(c) => {
                    self.line.insert(self.line_cursor, c);
                    self.line_cursor += c.len_utf8();
                }
                None => edited = false,
            },
        }
        if edited && (self.line.len(), self.line_cursor) != before {
            self.history_idx = None;
        }
        self.refresh_completion();
        self.dirty = true;
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.line[..self.line_cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.line[self.line_cursor..]
            .chars()
            .next()
            .map(|c| self.line_cursor + c.len_utf8())
    }

    /// Insert the ghost text, if any. The cursor has to be at the end.
    fn accept_ghost(&mut self) -> bool {
        if self.line_cursor != self.line.len() || self.completion.sugg.ghost.is_empty() {
            return false;
        }
        let ghost = std::mem::take(&mut self.completion.sugg.ghost);
        self.line.push_str(&ghost);
        self.line_cursor = self.line.len();
        true
    }

    /// Recompute suggestions for the current line (command mode only).
    pub(super) fn refresh_completion(&mut self) {
        if self.mode != UiMode::Command {
            self.completion.sugg = Suggestions::default();
            return;
        }
        let comp = &self.completion;
        let list = |d: &Path| comp.list_dir(d);
        let ctx = complete::Ctx {
            history: &self.history_cmd,
            palette: &self.settings.palette,
            list_dir: &list,
        };
        let sugg = complete::suggest(&self.line, self.line_cursor, &ctx);
        self.completion.sugg = sugg;
    }

    fn tab(&mut self, backward: bool) {
        let c = &mut self.completion;
        if let Some((head, tail)) = &c.cycle {
            let n = c.sugg.items.len();
            if n == 0 {
                return;
            }
            let i = match (c.selected, backward) {
                (Some(i), false) => (i + 1) % n,
                (Some(i), true) => (i + n - 1) % n,
                (None, false) => 0,
                (None, true) => n - 1,
            };
            c.selected = Some(i);
            let text = &c.sugg.items[i].text;
            self.line = format!("{head}{text}{tail}");
            self.line_cursor = head.len() + text.len();
            return;
        }

        let start = c.sugg.word_start.min(self.line_cursor);
        let typed = self.line[start..self.line_cursor].to_string();
        let items = &c.sugg.items;
        if items.is_empty() {
            return;
        }
        if let Some(prefix) = complete::tab_prefix(items, &typed) {
            let mut insert = prefix;
            // A single command that takes an argument: go on to it.
            if items.len() == 1 && items[0].wants_arg && insert == items[0].text {
                insert.push(' ');
            }
            self.line.replace_range(start..self.line_cursor, &insert);
            self.line_cursor = start + insert.len();
            self.refresh_completion();
            return;
        }
        if items.len() == 1 {
            let insert = items[0].text.clone();
            if insert != typed {
                self.line.replace_range(start..self.line_cursor, &insert);
                self.line_cursor = start + insert.len();
                self.refresh_completion();
            }
            return;
        }
        // Nothing in common to add: cycle through the list.
        let head = self.line[..start].to_string();
        let tail = self.line[self.line_cursor..].to_string();
        c.cycle = Some((head, tail));
        c.selected = None;
        self.tab(backward);
    }

    /// Walk the history. Only entries that start with the text before the
    /// cursor are visited (like Vim).
    fn history_step(&mut self, older: bool) {
        let prefix = match self.history_idx {
            Some(_) => self.history_prefix.clone(),
            None => self.line[..self.line_cursor].to_string(),
        };
        let matches = |i: &usize| self.history_cmd[*i].starts_with(&prefix);
        let n = self.history_cmd.len();
        let next = match (self.history_idx, older) {
            (None, true) => (0..n).rev().find(matches),
            (Some(i), true) => (0..i).rev().find(matches),
            (Some(i), false) => (i + 1..n).find(matches),
            (None, false) => return,
        };
        match next {
            Some(i) => {
                if self.history_idx.is_none() {
                    self.history_prefix = prefix;
                }
                self.history_idx = Some(i);
                self.line = self.history_cmd[i].clone();
            }
            None if !older && self.history_idx.is_some() => {
                // Past the newest entry: back to what was typed.
                self.history_idx = None;
                self.line = std::mem::take(&mut self.history_prefix);
            }
            None => return,
        }
        self.line_cursor = self.line.len();
    }

    /// The input line as shown: the text with a cursor mark, and the ghost.
    pub(super) fn line_display(&self) -> (String, String) {
        let (a, b) = self.line.split_at(self.line_cursor.min(self.line.len()));
        (format!("{a}▏{b}"), self.completion.sugg.ghost.clone())
    }
}
