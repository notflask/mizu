//! Search state and the replies from the service thread.

use super::{Pos, Viewer};
use crate::doc::service::{Job, Reply};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub page: usize,
    pub rect: [f32; 4],
}

#[derive(Default)]
pub struct Search {
    pub id: u64,
    pub needle: String,
    pub forward: bool,
    /// Sorted by page, then position.
    pub hits: Vec<Hit>,
    pub current: Option<usize>,
    pub running: bool,
    pub visible: bool,
    start: Option<Pos>,
    pushed_jump: bool,
}

fn hit_key(h: &Hit) -> (usize, i64, i64) {
    (
        h.page,
        (h.rect[1] * 100.0) as i64,
        (h.rect[0] * 100.0) as i64,
    )
}

impl Search {
    fn insert(&mut self, hit: Hit) {
        let k = hit_key(&hit);
        let at = self.hits.partition_point(|h| hit_key(h) < k);
        self.hits.insert(at, hit);
        if let Some(c) = &mut self.current {
            if at <= *c {
                *c += 1;
            }
        }
    }
}

impl Viewer {
    pub fn start_search(&mut self, text: &str, forward: bool) {
        let text = text.trim_end_matches('\r');
        if text.is_empty() {
            return;
        }
        let start = self.top_pos();
        let Some(d) = &mut self.doc else { return };
        // Smartcase: any capital letter makes the search case-sensitive.
        let case_sensitive = text.chars().any(|c| c.is_uppercase());
        let id = d.search.id + 1;
        let start_page = start.map(|s| s.page).unwrap_or(0);
        d.search = Search {
            id,
            needle: text.to_string(),
            forward,
            hits: Vec::new(),
            current: None,
            running: true,
            visible: true,
            start,
            pushed_jump: false,
        };
        d.service.send(Job::Search {
            id,
            needle: text.to_string(),
            case_sensitive,
            start_page,
            forward,
        });
        self.dirty = true;
    }

    pub fn search_step(&mut self, same_direction: bool) {
        let Some(d) = &mut self.doc else { return };
        if d.search.hits.is_empty() {
            if d.search.needle.is_empty() {
                self.error("E35: No previous regular expression");
            } else if !d.search.running {
                let n = d.search.needle.clone();
                self.error(format!("E486: Pattern not found: {n}"));
            }
            return;
        }
        d.search.visible = true;
        let fwd = d.search.forward == same_direction;
        let n = d.search.hits.len();
        let cur = d.search.current.unwrap_or(if fwd { n - 1 } else { 0 });
        let next = if fwd {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        d.search.current = Some(next);
        self.show_hit(next);
    }

    fn show_hit(&mut self, i: usize) {
        let Some(d) = &mut self.doc else { return };
        let Some(h) = d.search.hits.get(i).copied() else {
            return;
        };
        if !d.search.pushed_jump {
            d.search.pushed_jump = true;
            if let Some(start) = d.search.start {
                d.jumps.truncate(d.jump_idx);
                if d.jumps.last() != Some(&start) {
                    d.jumps.push(start);
                }
                d.jump_idx = d.jumps.len();
            }
        }
        let g = d.layout.pages[h.page];
        let s = self.camera.scale();
        let view_h = self.camera.viewport[1] / s;
        let view_w = self.camera.viewport[0] / s;
        let cy = g.y + (h.rect[1] + h.rect[3]) * 0.5;
        let cx = g.x + (h.rect[0] + h.rect[2]) * 0.5;
        let mut target = [self.camera.offset[0], cy - view_h * 0.5];
        if cx < self.camera.offset[0] + view_w * 0.1 || cx > self.camera.offset[0] + view_w * 0.9 {
            target[0] = cx - view_w * 0.5;
        }
        self.camera_set_target(target);
        self.dirty = true;
    }

    fn camera_set_target(&mut self, target: [f32; 2]) {
        if let Some(d) = &self.doc {
            self.anim_target = Some(self.camera.clamped(&d.layout, target));
        }
    }

    pub(super) fn poll_service(&mut self) {
        let mut replies = Vec::new();
        if let Some(d) = &self.doc {
            while let Ok(r) = d.service.rx.try_recv() {
                replies.push(r);
            }
        }
        for r in replies {
            match r {
                Reply::SearchPage { id, page, hits } => self.on_search_page(id, page, hits),
                Reply::SearchDone { id } => self.on_search_done(id),
                Reply::Links { page, links } => {
                    if let Some(d) = &mut self.doc {
                        d.links.insert(page, links);
                    }
                }
                Reply::Saved { id, dst, result } => self.on_saved(id, dst, result),
            }
            self.dirty = true;
        }
    }

    fn on_search_page(&mut self, id: u64, page: usize, hits: Vec<[f32; 4]>) {
        let cursor = self.top_pos();
        let Some(d) = &mut self.doc else { return };
        if d.search.id != id {
            return;
        }
        let first_batch = d.search.hits.is_empty();
        let mut sorted: Vec<Hit> = hits.into_iter().map(|rect| Hit { page, rect }).collect();
        sorted.sort_by_key(hit_key);
        let start = d.search.start.or(cursor);
        let forward = d.search.forward;
        // Pick the first hit after (before) the starting point as soon as it shows up.
        let mut pick: Option<Hit> = None;
        if d.search.current.is_none() {
            pick = match (forward, start) {
                (true, Some(s)) if s.page == page => {
                    sorted.iter().find(|h| h.rect[3] > s.y + 4.0).copied()
                }
                (false, Some(s)) if s.page == page => {
                    sorted.iter().rev().find(|h| h.rect[1] < s.y - 4.0).copied()
                }
                (true, _) => sorted.first().copied(),
                (false, _) => sorted.last().copied(),
            };
        }
        for h in sorted {
            d.search.insert(h);
        }
        if let Some(h) = pick {
            let idx = d.search.hits.iter().position(|x| *x == h);
            d.search.current = idx;
            if let Some(i) = idx {
                self.show_hit(i);
            }
        }
        let _ = first_batch;
    }

    fn on_search_done(&mut self, id: u64) {
        let Some(d) = &mut self.doc else { return };
        if d.search.id != id {
            return;
        }
        d.search.running = false;
        if d.search.hits.is_empty() {
            let n = d.search.needle.clone();
            self.error(format!("E486: Pattern not found: {n}"));
            return;
        }
        if d.search.current.is_none() {
            // Everything was behind the starting point: wrap around.
            let i = if d.search.forward {
                0
            } else {
                d.search.hits.len() - 1
            };
            d.search.current = Some(i);
            self.show_hit(i);
            self.info("search hit BOTTOM, continuing at TOP");
        }
    }

    pub(super) fn on_saved(
        &mut self,
        id: u64,
        dst: std::path::PathBuf,
        result: Result<(), String>,
    ) {
        let Some(d) = &mut self.doc else { return };
        let Some(s) = d.saving.take() else { return };
        if s.id != id {
            d.saving = Some(s);
            return;
        }
        let name = dst
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match result {
            Ok(()) => {
                if dst == d.path {
                    if d.history.revision() == s.revision {
                        d.history.mark_saved();
                    }
                    d.mtime = std::fs::metadata(&d.path).and_then(|m| m.modified()).ok();
                    self.info(format!("\"{name}\" written"));
                } else {
                    self.info(format!("written to \"{name}\""));
                }
                if s.quit_after {
                    self.quit_now();
                }
            }
            Err(e) => self.error(format!("E212: Can't write \"{name}\": {e}")),
        }
    }

    pub(super) fn poll_watcher(&mut self) {
        let mut changed = false;
        if let Some(d) = &self.doc {
            if let Some(rx) = &d.watcher_rx {
                while rx.try_recv().is_ok() {
                    changed = true;
                }
            }
        }
        if changed {
            self.external_change();
        }
    }
}
