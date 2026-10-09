//! Remembers where you were in each file (and your marks) between runs.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 500;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FileState {
    pub page: usize,
    /// Points between the top of `page` and the top of the viewport.
    pub y_in_page: f32,
    pub x: f32,
    pub zoom: f32,
    /// "fit_width" | "fit_page" | "free"
    pub mode: String,
    pub dark: Option<bool>,
    /// mark -> (page, y_in_page)
    pub marks: BTreeMap<char, (usize, f32)>,
    /// Monotonic counter used to evict the least recently used entries.
    pub stamp: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    files: BTreeMap<String, FileState>,
    counter: u64,
    /// Pen settings, remembered across files and runs.
    #[serde(default)]
    pub pen: Option<PenPrefs>,
    /// Inner window size in logical pixels.
    #[serde(default)]
    pub window: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct PenPrefs {
    pub color: [u8; 3],
    pub width: f32,
}

fn key_for(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

impl Session {
    pub fn path() -> Option<PathBuf> {
        crate::config::state_dir().map(|d| d.join("session.json"))
    }

    pub fn load() -> Session {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn get(&self, file: &Path) -> Option<&FileState> {
        self.files.get(&key_for(file))
    }

    pub fn set(&mut self, file: &Path, mut state: FileState) {
        self.counter += 1;
        state.stamp = self.counter;
        self.files.insert(key_for(file), state);
        if self.files.len() > MAX_ENTRIES {
            let mut stamps: Vec<(u64, String)> = self
                .files
                .iter()
                .map(|(k, v)| (v.stamp, k.clone()))
                .collect();
            stamps.sort();
            let excess = self.files.len() - MAX_ENTRIES;
            for (_, k) in stamps.into_iter().take(excess) {
                self.files.remove(&k);
            }
        }
    }

    /// Files that still exist, most recently used first.
    pub fn recent(&self, limit: usize) -> Vec<(PathBuf, &FileState)> {
        let mut v: Vec<(&String, &FileState)> = self.files.iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1.stamp));
        v.into_iter()
            .map(|(k, st)| (PathBuf::from(k), st))
            .filter(|(p, _)| p.is_file())
            .take(limit)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Atomic write. Errors are returned, callers usually just log them.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = Self::path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        let json = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        tmp.write_all(&json)?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(page: usize) -> FileState {
        FileState {
            page,
            zoom: 1.5,
            mode: "free".into(),
            ..Default::default()
        }
    }

    #[test]
    fn roundtrip_via_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.pdf");
        std::fs::write(&file, b"x").unwrap();
        let mut s = Session::default();
        let mut state = st(7);
        state.marks.insert('a', (3, 12.5));
        state.dark = Some(true);
        s.set(&file, state.clone());
        let path = dir.path().join("session.json");
        s.save_to(&path).unwrap();

        let loaded: Session =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let got = loaded.get(&file).unwrap();
        assert_eq!(got.page, 7);
        assert_eq!(got.marks.get(&'a'), Some(&(3, 12.5)));
        assert_eq!(got.dark, Some(true));
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut s = Session::default();
        for i in 0..(MAX_ENTRIES + 20) {
            s.set(Path::new(&format!("/nonexistent/file-{i}.pdf")), st(i));
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        assert!(s.get(Path::new("/nonexistent/file-0.pdf")).is_none());
        assert!(s
            .get(Path::new(&format!(
                "/nonexistent/file-{}.pdf",
                MAX_ENTRIES + 19
            )))
            .is_some());
    }

    #[test]
    fn updating_refreshes_recency() {
        let mut s = Session::default();
        s.set(Path::new("/nonexistent/keep.pdf"), st(1));
        for i in 0..MAX_ENTRIES {
            s.set(Path::new(&format!("/nonexistent/f-{i}.pdf")), st(i));
            if i % 100 == 0 {
                s.set(Path::new("/nonexistent/keep.pdf"), st(2));
            }
        }
        assert!(s.get(Path::new("/nonexistent/keep.pdf")).is_some());
    }

    #[test]
    fn garbage_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("session.json");
        std::fs::write(&p, "not json").unwrap();
        let loaded: Option<Session> =
            serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).ok();
        assert!(loaded.is_none());
    }

    #[test]
    fn old_files_without_prefs_still_load() {
        let s: Session = serde_json::from_str(r#"{"files":{},"counter":3}"#).unwrap();
        assert_eq!(s.pen, None);
        assert_eq!(s.window, None);
    }

    #[test]
    fn recent_lists_existing_files_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.pdf");
        let b = dir.path().join("b.pdf");
        let gone = dir.path().join("gone.pdf");
        for f in [&a, &b, &gone] {
            std::fs::write(f, b"x").unwrap();
        }
        let mut s = Session::default();
        s.set(&a, st(1));
        s.set(&gone, st(2));
        s.set(&b, st(3));
        std::fs::remove_file(&gone).unwrap();
        let r = s.recent(10);
        let names: Vec<_> = r
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, vec!["b.pdf", "a.pdf"]);
    }
}
