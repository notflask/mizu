//! Watches the directory of the open file so LaTeX-style "write a temp file
//! and rename it over the PDF" updates are noticed.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver};
use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, Debouncer};

pub struct Watcher {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    pub rx: Receiver<()>,
}

/// Start watching `file`. Returns `None` if the platform refuses (the viewer
/// still works, it just will not reload by itself).
pub fn watch(file: &Path, wake: Arc<dyn Fn() + Send + Sync>) -> Option<Watcher> {
    let target: PathBuf = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    let dir = target.parent()?.to_path_buf();
    let (tx, rx) = unbounded();
    let t2 = target.clone();
    let mut debouncer = new_debouncer(
        Duration::from_millis(200),
        move |res: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = res {
                let hit = events.iter().any(|e| {
                    e.path == t2
                        || std::fs::canonicalize(&e.path)
                            .map(|p| p == t2)
                            .unwrap_or_else(|_| e.path.file_name() == t2.file_name())
                });
                if hit {
                    let _ = tx.send(());
                    wake();
                }
            }
        },
    )
    .ok()?;
    debouncer
        .watcher()
        .watch(&dir, RecursiveMode::NonRecursive)
        .ok()?;
    Some(Watcher {
        _debouncer: debouncer,
        rx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn notices_rename_over_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("doc.pdf");
        std::fs::write(&file, b"one").unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let w = watch(
            &file,
            Arc::new(move || {
                h.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .expect("watcher");
        std::thread::sleep(Duration::from_millis(300));
        // LaTeX style: write a new file, rename it over the old one.
        let tmp = dir.path().join("doc.pdf.tmp");
        std::fs::write(&tmp, b"two").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        w.rx.recv_timeout(Duration::from_secs(5))
            .expect("no event after rename");
        assert!(hits.load(Ordering::SeqCst) >= 1);
        // Unrelated files do not trigger.
        while w.rx.try_recv().is_ok() {}
        std::fs::write(dir.path().join("other.txt"), b"x").unwrap();
        assert!(w.rx.recv_timeout(Duration::from_millis(800)).is_err());
    }
}
