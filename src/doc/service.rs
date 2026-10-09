//! One background thread for everything that is not tile rendering:
//! search, saving and link extraction. It owns its own MuPDF document.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};
use mupdf::{DestinationKind, Document, TextPageFlags};

use super::{open_document, OpenDoc, Reflow};
use crate::ink::Stroke;

#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Page { page: usize, y: Option<f32> },
    Uri(String),
}

#[derive(Clone, Debug)]
pub struct LinkInfo {
    /// `[x0, y0, x1, y1]` in page space.
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

pub enum Job {
    Search {
        id: u64,
        needle: String,
        case_sensitive: bool,
        start_page: usize,
        forward: bool,
    },
    Save {
        id: u64,
        dst: PathBuf,
        strokes: Vec<Stroke>,
    },
    Links {
        pages: Vec<usize>,
    },
}

pub enum Reply {
    /// Hits for one page (page space rects), in search order.
    SearchPage {
        id: u64,
        page: usize,
        hits: Vec<[f32; 4]>,
    },
    SearchDone {
        id: u64,
    },
    Saved {
        id: u64,
        dst: PathBuf,
        result: Result<(), String>,
    },
    Links {
        page: usize,
        links: Vec<LinkInfo>,
    },
}

pub struct Service {
    tx: Sender<Job>,
    pub rx: Receiver<Reply>,
    /// Id of the search that should keep running; older ones stop early.
    current_search: Arc<AtomicU64>,
    handle: Option<JoinHandle<()>>,
}

impl Service {
    pub fn spawn(
        path: PathBuf,
        password: Option<String>,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Service {
        Self::spawn_reflow(path, password, Reflow::default(), wake)
    }

    pub fn spawn_reflow(
        path: PathBuf,
        password: Option<String>,
        reflow: Reflow,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> Service {
        let (tx, jobs) = unbounded::<Job>();
        let (reply_tx, rx) = unbounded::<Reply>();
        let current_search = Arc::new(AtomicU64::new(0));
        let cs = current_search.clone();
        let handle = std::thread::Builder::new()
            .name("mizu-service".into())
            .spawn(move || run(path, password, reflow, jobs, reply_tx, cs, wake))
            .ok();
        Service {
            tx,
            rx,
            current_search,
            handle,
        }
    }

    pub fn send(&self, job: Job) {
        if let Job::Search { id, .. } = &job {
            self.current_search.store(*id, Ordering::SeqCst);
        }
        let _ = self.tx.send(job);
    }

    pub fn cancel_search(&self) {
        self.current_search.store(0, Ordering::SeqCst);
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.current_search.store(0, Ordering::SeqCst);
        // Dropping the sender ends the thread's loop. Do not join: a long
        // running save should finish in the background.
        let (dead_tx, _) = unbounded();
        self.tx = dead_tx;
        self.handle.take();
    }
}

fn run(
    path: PathBuf,
    password: Option<String>,
    reflow: Reflow,
    jobs: Receiver<Job>,
    out: Sender<Reply>,
    current_search: Arc<AtomicU64>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut doc: Option<OpenDoc> = None;
    let ensure = |doc: &mut Option<OpenDoc>| -> bool {
        if doc.is_none() {
            *doc = open_document(&path, password.as_deref(), reflow).ok();
        }
        doc.is_some()
    };
    let send = |r: Reply| {
        let _ = out.send(r);
        wake();
    };

    while let Ok(job) = jobs.recv() {
        match job {
            Job::Search {
                id,
                needle,
                case_sensitive,
                start_page,
                forward,
            } => {
                if !ensure(&mut doc) {
                    send(Reply::SearchDone { id });
                    continue;
                }
                // Image books have no text to search.
                let Some(d) = doc.as_ref().and_then(|d| d.doc()) else {
                    send(Reply::SearchDone { id });
                    continue;
                };
                let n = d.page_count().unwrap_or(0).max(0) as usize;
                for step in 0..n {
                    if current_search.load(Ordering::SeqCst) != id {
                        break;
                    }
                    let page = if forward {
                        (start_page + step) % n
                    } else {
                        (start_page + n - (step % n)) % n
                    };
                    let hits = search_page(d, page, &needle, case_sensitive);
                    if !hits.is_empty() {
                        send(Reply::SearchPage { id, page, hits });
                    }
                }
                send(Reply::SearchDone { id });
            }
            Job::Save { id, dst, strokes } => {
                let result =
                    super::annots::save_with_strokes(&path, &dst, password.as_deref(), &strokes);
                send(Reply::Saved { id, dst, result });
            }
            Job::Links { pages } => {
                if !ensure(&mut doc) {
                    continue;
                }
                let Some(d) = doc.as_ref().and_then(|d| d.doc()) else {
                    continue;
                };
                for p in pages {
                    let links = page_links(d, p);
                    send(Reply::Links { page: p, links });
                }
            }
        }
    }
}

/// Search one page. MuPDF's search ignores case; for case-sensitive queries
/// the hits are filtered against the page text.
pub fn search_page(
    doc: &Document,
    page: usize,
    needle: &str,
    case_sensitive: bool,
) -> Vec<[f32; 4]> {
    let Ok(p) = doc.load_page(page as i32) else {
        return Vec::new();
    };
    let Ok(quads) = p.search(needle, 512) else {
        return Vec::new();
    };
    let mut rects: Vec<[f32; 4]> = quads
        .iter()
        .map(|q| {
            let xs = [q.ul.x, q.ur.x, q.ll.x, q.lr.x];
            let ys = [q.ul.y, q.ur.y, q.ll.y, q.lr.y];
            [
                xs.iter().cloned().fold(f32::MAX, f32::min),
                ys.iter().cloned().fold(f32::MAX, f32::min),
                xs.iter().cloned().fold(f32::MIN, f32::max),
                ys.iter().cloned().fold(f32::MIN, f32::max),
            ]
        })
        .collect();
    if case_sensitive && !rects.is_empty() {
        if let Ok(tp) = p.to_text_page(TextPageFlags::empty()) {
            if let Ok(text) = tp.to_text() {
                rects = filter_case(rects, &text, needle);
            }
        }
    }
    rects
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Keep the hits whose underlying text matches `needle` exactly. Hits and
/// text occurrences are both in reading order, so they pair up by index; if
/// the counts disagree we cannot pair them and keep everything that has at
/// least one exact match on the page.
pub fn filter_case(rects: Vec<[f32; 4]>, text: &str, needle: &str) -> Vec<[f32; 4]> {
    let text = collapse_ws(text);
    let needle = collapse_ws(needle);
    if needle.is_empty() {
        return rects;
    }
    let lower_text = text.to_lowercase();
    let lower_needle = needle.to_lowercase();
    if lower_text.len() != text.len() || lower_needle.len() != needle.len() {
        // Case mapping changed byte lengths; do not try to be clever.
        return if text.contains(&needle) {
            rects
        } else {
            Vec::new()
        };
    }
    let mut exact = Vec::new();
    let mut from = 0;
    while let Some(i) = lower_text[from..].find(&lower_needle) {
        let at = from + i;
        exact.push(text[at..at + needle.len()] == *needle);
        from = at + lower_needle.len().max(1);
    }
    if exact.len() == rects.len() {
        rects
            .into_iter()
            .zip(exact)
            .filter_map(|(r, ok)| ok.then_some(r))
            .collect()
    } else if exact.iter().any(|&b| b) {
        rects
    } else {
        Vec::new()
    }
}

fn page_links(doc: &Document, page: usize) -> Vec<LinkInfo> {
    let Ok(p) = doc.load_page(page as i32) else {
        return Vec::new();
    };
    let Ok(iter) = p.links() else {
        return Vec::new();
    };
    iter.filter_map(|l| {
        let rect = [l.bounds.x0, l.bounds.y0, l.bounds.x1, l.bounds.y1];
        let target = match l.dest {
            Some(d) => {
                let y = match d.kind {
                    DestinationKind::XYZ { top, .. } | DestinationKind::FitH { top } => top,
                    _ => None,
                };
                LinkTarget::Page {
                    page: d.loc.page_number as usize,
                    y,
                }
            }
            None if !l.uri.is_empty() => LinkTarget::Uri(l.uri),
            None => return None,
        };
        Some(LinkInfo { rect, target })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_filter_pairs_hits_by_order() {
        let rects = vec![[0.0; 4], [1.0; 4], [2.0; 4]];
        // "Rust", "rust", "RUST" -> only the first one matches "Rust".
        let out = filter_case(rects, "Rust is rust and RUST", "Rust");
        assert_eq!(out, vec![[0.0; 4]]);
    }

    #[test]
    fn case_filter_handles_whitespace_runs() {
        let rects = vec![[5.0; 4]];
        let out = filter_case(rects, "neural\n  network", "neural network");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn case_filter_no_exact_match() {
        let out = filter_case(vec![[0.0; 4]], "rust", "Rust");
        assert!(out.is_empty());
    }

    #[test]
    fn case_filter_count_mismatch_keeps_all_when_something_matches() {
        let rects = vec![[0.0; 4], [1.0; 4], [2.0; 4]];
        let out = filter_case(rects.clone(), "Rust rust", "Rust");
        assert_eq!(out, rects);
    }
}
