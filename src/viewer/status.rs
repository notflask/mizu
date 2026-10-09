//! What the status line says, and the highlight list for the renderer.

use super::{Tool, UiMode, Viewer};
use crate::render::ui::{ListOverlay, UiState};
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
            (UiMode::Outline, _) => "OUTLINE",
            (UiMode::Password, _) => "PASSWORD",
            _ => "NORMAL",
        };

        // Input line (":", "/", "?" or the masked password prompt).
        st.input = match self.mode {
            UiMode::Command => Some(format!(":{}▏", self.line)),
            UiMode::Search { forward } => {
                Some(format!("{}{}▏", if forward { '/' } else { '?' }, self.line))
            }
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

        if self.mode == UiMode::Draw {
            let text = match self.tool {
                Tool::Pen => format!("{:.1}pt", self.pen_width),
                Tool::Eraser => "eraser".to_string(),
            };
            st.pen = Some((self.pen_color, text));
        }

        if self.mode == UiMode::Outline {
            let vis = self.outline_visible();
            let lines: Vec<String> = vis
                .iter()
                .map(|(_, o)| {
                    let indent = "  ".repeat(o.level as usize);
                    match o.page {
                        Some(p) => format!("{indent}{}  ·  {}", o.title, p + 1),
                        None => format!("{indent}{}", o.title),
                    }
                })
                .collect();
            let title = if self.outline_filter.is_empty() && !self.outline_filtering {
                "Outline   (j/k move · / filter · Enter jump · Esc close)".to_string()
            } else {
                format!("Outline  /{}", self.outline_filter)
            };
            st.overlay = Some(ListOverlay {
                title,
                selected: self.outline_sel.min(lines.len().saturating_sub(1)),
                lines,
            });
        }

        if self.doc.is_none() && !self.is_loading() && self.mode != UiMode::Password {
            st.hint = Some("mizu — :e <file>".into());
        }
        st
    }
}
