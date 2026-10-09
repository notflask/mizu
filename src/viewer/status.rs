//! What the status line says, and the highlight list for the renderer.

use super::{ListKind, Tool, UiMode, Viewer};
use crate::render::recolor::{recolor_srgb8, DarkTheme};
use crate::render::ui::{ListOverlay, PenUi, Popup, PopupRow, Titlebar, UiState};
use crate::render::Highlight;

impl Viewer {
    pub fn refresh_highlights(&mut self) {
        self.highlights.clear();
        let Some(d) = &self.doc else { return };
        if !d.search.visible || d.search.hits.is_empty() {
            return;
        }
        let [_, y0, _, y1] = self.camera.visible_doc_rect();
        let vis = d.layout.visible(y0, y1);
        let hits = &d.search.hits;
        let from = hits.partition_point(|h| h.page < vis.start);
        for (i, h) in hits.iter().enumerate().skip(from) {
            if h.page >= vis.end {
                break;
            }
            self.highlights.push(Highlight {
                page: h.page,
                rect: h.rect,
                current: d.search.current == Some(i),
            });
        }
    }

    /// A pen colour as it looks on the page right now (recoloured in dark
    /// mode, like the ink itself).
    pub fn display_color(&self, c: [u8; 3]) -> [u8; 3] {
        if self.dark {
            let t = DarkTheme::from_srgb8(self.theme.dark_fg, self.theme.dark_bg);
            recolor_srgb8(c, &t)
        } else {
            c
        }
    }

    pub fn ui_state(&self) -> UiState {
        let mut st = UiState {
            statusbar: self.settings.statusbar,
            dark: self.dark,
            ..Default::default()
        };
        let mode_label = match (self.mode, self.tool) {
            (UiMode::Draw, Tool::Eraser) => "ERASE",
            (UiMode::Draw, Tool::Pen) => "DRAW",
            (UiMode::Command, _) => "COMMAND",
            (UiMode::Search { .. }, _) => "SEARCH",
            (UiMode::Outline, _) => match self.list_kind {
                ListKind::Outline => "OUTLINE",
                ListKind::Help => "HELP",
                ListKind::Recent => "RECENT",
            },
            (UiMode::Password, _) => "PASSWORD",
            _ => "NORMAL",
        };

        // Input line (":", "/", "?" or the masked password prompt).
        st.input = match self.mode {
            UiMode::Command => Some(format!(":{}", self.line_display().0)),
            UiMode::Search { forward } => Some(format!(
                "{}{}",
                if forward { '/' } else { '?' },
                self.line_display().0
            )),
            UiMode::Password => Some(format!(
                "Password: {}▏",
                "*".repeat(self.line.chars().count())
            )),
            _ => None,
        };

        let name = self
            .doc
            .as_ref()
            .and_then(|d| d.path.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(m) = &self.message {
            st.left = m.text.clone();
            st.left_is_error = m.error;
        } else if self.is_loading() && self.doc.is_none() {
            st.left = "Opening…".into();
        } else {
            let mut left = mode_label.to_string();
            if let Some(n) = &name {
                left.push_str("  ");
                left.push_str(n);
            }
            if self.is_dirty() {
                left.push_str(" [+]");
            }
            st.left = left;
        }

        let mut right = String::new();
        if self.show_stats && !self.stats_text.is_empty() {
            right.push_str(&self.stats_text);
            right.push_str("  ");
        }
        let pending = self.engine.pending_text();
        if !pending.is_empty() {
            right.push_str(&pending);
            right.push_str("  ");
        }
        if let Some(d) = &self.doc {
            if d.search.running {
                right.push_str("searching…  ");
            } else if let (Some(c), true) = (d.search.current, d.search.visible) {
                right.push_str(&format!("[{}/{}]  ", c + 1, d.search.hits.len()));
            }
            right.push_str(&format!(
                "{}/{}  {:.0}%",
                self.current_page() + 1,
                d.layout.len(),
                self.zoom_percent()
            ));
        }
        st.right = right;

        if self.mode == UiMode::Command {
            st.ghost = self.line_display().1;
            let sg = &self.completion.sugg;
            if !sg.items.is_empty() {
                st.popup = Some(Popup {
                    rows: sg
                        .items
                        .iter()
                        .map(|c| PopupRow {
                            label: c.label.clone(),
                            detail: match (c.hint.is_empty(), c.help.is_empty()) {
                                (false, false) => format!("{}  {}", c.hint, c.help),
                                (false, true) => c.hint.clone(),
                                _ => c.help.clone(),
                            },
                            swatch: c.swatch.map(|x| self.display_color(x)),
                        })
                        .collect(),
                    selected: self.completion.selected,
                    // One column for the ":".
                    column: 1 + self.line[..sg.word_start.min(self.line.len())]
                        .chars()
                        .count(),
                });
            }
        }

        if self.mode == UiMode::Draw {
            let text = match self.tool {
                Tool::Pen => format!("{:.1}pt", self.pen_width),
                Tool::Eraser => "eraser".to_string(),
            };
            st.pen = Some(PenUi {
                palette: self
                    .settings
                    .palette
                    .iter()
                    .map(|c| self.display_color(*c))
                    .collect(),
                selected: self
                    .settings
                    .palette
                    .iter()
                    .position(|c| *c == self.pen_color),
                color: self.display_color(self.pen_color),
                text,
            });
        }

        if self.mode == UiMode::Outline {
            let lines: Vec<String> = self.list_visible().into_iter().map(|(_, t)| t).collect();
            let name = match self.list_kind {
                ListKind::Outline => "Outline",
                ListKind::Help => "Help",
                ListKind::Recent => "Recent files",
            };
            let title = if self.outline_filter.is_empty() && !self.outline_filtering {
                format!("{name}   (j/k move · / filter · Enter open · Esc close)")
            } else {
                format!("{name}  /{}", self.outline_filter)
            };
            st.overlay = Some(ListOverlay {
                title,
                selected: self.outline_sel.min(lines.len().saturating_sub(1)),
                lines,
            });
        }

        if self.camera.top_inset > 0.0 {
            st.titlebar = Some(Titlebar {
                height: self.camera.top_inset,
                text: match &name {
                    Some(n) if self.is_dirty() => format!("{n}  ●"),
                    Some(n) => n.clone(),
                    None => "mizu".into(),
                },
            });
        }

        if self.doc.is_none() && !self.is_loading() && self.mode != UiMode::Password {
            st.hint = Some("mizu — :e <file>   ·   :recent".into());
        }
        st
    }
}
