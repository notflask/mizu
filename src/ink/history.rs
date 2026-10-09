//! Undo / redo and the "unsaved changes" flag.

use super::store::Store;
use super::stroke::Stroke;

#[derive(Clone, Debug)]
pub enum Edit {
    Add(Stroke),
    /// Strokes removed in one eraser drag, with their original indices.
    Remove(Vec<(usize, Stroke)>),
}

impl Edit {
    pub fn page(&self) -> Option<usize> {
        match self {
            Edit::Add(s) => Some(s.page),
            Edit::Remove(v) => v.first().map(|(_, s)| s.page),
        }
    }
}

#[derive(Debug)]
pub struct History {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Length of the undo stack at the time of the last save; `None` once
    /// the saved state is no longer reachable.
    saved_len: Option<usize>,
    /// Bumped by every change; lets an async save notice later edits.
    revision: u64,
}

impl Default for History {
    fn default() -> Self {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            saved_len: Some(0),
            revision: 0,
        }
    }
}

impl History {
    pub fn is_dirty(&self) -> bool {
        self.saved_len != Some(self.undo.len())
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_len = Some(self.undo.len());
    }

    fn record(&mut self, edit: Edit) {
        // A new edit throws away the redo branch; if the saved state lived
        // there it can never be reached again.
        if let Some(l) = self.saved_len {
            if l > self.undo.len() {
                self.saved_len = None;
            }
        }
        self.redo.clear();
        self.undo.push(edit);
        self.revision += 1;
    }

    pub fn add_stroke(&mut self, store: &mut Store, stroke: Stroke) {
        store.push(stroke.clone());
        self.record(Edit::Add(stroke));
    }

    /// Remove the given strokes in one undoable step. Returns how many
    /// strokes were actually removed.
    pub fn remove_strokes(&mut self, store: &mut Store, page: usize, ids: &[uuid::Uuid]) -> usize {
        let mut removed: Vec<(usize, Stroke)> = Vec::new();
        for &id in ids {
            if let Some(r) = store.remove_by_id(page, id) {
                removed.push(r);
            }
        }
        self.record_removed(removed)
    }

    /// Record strokes that were already taken out of the store (one eraser
    /// drag, possibly across pages) as a single undo step. Each entry holds
    /// the index the stroke had *at the time it was removed*, in removal
    /// order. Returns the number of strokes.
    pub fn record_removed(&mut self, removed: Vec<(usize, Stroke)>) -> usize {
        let n = removed.len();
        if n == 0 {
            return 0;
        }
        // Convert every index to the state before the first removal on its
        // page, so that undo can re-insert in ascending order.
        let raw: Vec<usize> = removed.iter().map(|(i, _)| *i).collect();
        let mut orig: Vec<(usize, Stroke)> = Vec::with_capacity(n);
        for (i, (_, s)) in removed.iter().enumerate() {
            let mut x = raw[i];
            for j in (0..i).rev() {
                if removed[j].1.page == s.page && x >= raw[j] {
                    x += 1;
                }
            }
            orig.push((x, s.clone()));
        }
        orig.sort_by_key(|(i, s)| (s.page, *i));
        self.record(Edit::Remove(orig));
        n
    }

    /// Returns the page that changed, if anything was undone.
    pub fn undo(&mut self, store: &mut Store) -> Option<usize> {
        let edit = self.undo.pop()?;
        let page = edit.page();
        match &edit {
            Edit::Add(s) => {
                store.remove_by_id(s.page, s.id);
            }
            Edit::Remove(items) => {
                for (idx, s) in items {
                    store.insert(*idx, s.clone());
                }
            }
        }
        self.redo.push(edit);
        self.revision += 1;
        page
    }

    pub fn redo(&mut self, store: &mut Store) -> Option<usize> {
        let edit = self.redo.pop()?;
        let page = edit.page();
        match &edit {
            Edit::Add(s) => store.push(s.clone()),
            Edit::Remove(items) => {
                for (_, s) in items {
                    store.remove_by_id(s.page, s.id);
                }
            }
        }
        self.undo.push(edit);
        self.revision += 1;
        page
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(y: f32) -> Stroke {
        Stroke::new(0, vec![[0.0, y], [10.0, y]], None, 1.0, [0; 3])
    }

    #[test]
    fn undo_redo_add() {
        let mut st = Store::new(1);
        let mut h = History::default();
        assert!(!h.is_dirty());
        h.add_stroke(&mut st, line(1.0));
        h.add_stroke(&mut st, line(2.0));
        assert_eq!(st.total(), 2);
        assert!(h.is_dirty());
        assert_eq!(h.undo(&mut st), Some(0));
        assert_eq!(st.total(), 1);
        assert_eq!(h.redo(&mut st), Some(0));
        assert_eq!(st.total(), 2);
        assert_eq!(h.redo(&mut st), None);
    }

    #[test]
    fn erase_drag_is_one_undo_step() {
        let mut st = Store::new(1);
        let mut h = History::default();
        let strokes: Vec<_> = (0..5).map(|i| line(i as f32 * 10.0)).collect();
        let order: Vec<_> = strokes.iter().map(|s| s.id).collect();
        for s in strokes {
            h.add_stroke(&mut st, s);
        }
        // Erase #1 and #3 in one go.
        let n = h.remove_strokes(&mut st, 0, &[order[1], order[3]]);
        assert_eq!(n, 2);
        assert_eq!(st.total(), 3);
        h.undo(&mut st);
        assert_eq!(st.total(), 5);
        // Original draw order restored exactly.
        let ids: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        assert_eq!(ids, order);
        h.redo(&mut st);
        assert_eq!(st.total(), 3);
        assert!(!st
            .strokes(0)
            .iter()
            .any(|s| s.id == order[1] || s.id == order[3]));
    }

    #[test]
    fn erase_adjacent_strokes_restores_order() {
        let mut st = Store::new(1);
        let mut h = History::default();
        let strokes: Vec<_> = (0..4).map(|i| line(i as f32)).collect();
        let order: Vec<_> = strokes.iter().map(|s| s.id).collect();
        for s in strokes {
            h.add_stroke(&mut st, s);
        }
        h.remove_strokes(&mut st, 0, &[order[1], order[2]]);
        h.undo(&mut st);
        let ids: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        assert_eq!(ids, order);
        // Reverse eraser order must work too.
        h.redo(&mut st);
        h.undo(&mut st);
        h.remove_strokes(&mut st, 0, &[order[2], order[1]]);
        h.undo(&mut st);
        let ids: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        assert_eq!(ids, order);
    }

    #[test]
    fn dirty_flag_follows_the_saved_state() {
        let mut st = Store::new(1);
        let mut h = History::default();
        h.add_stroke(&mut st, line(1.0));
        h.mark_saved();
        assert!(!h.is_dirty());
        h.add_stroke(&mut st, line(2.0));
        assert!(h.is_dirty());
        h.undo(&mut st);
        assert!(!h.is_dirty()); // back at the saved state
        h.undo(&mut st);
        assert!(h.is_dirty());
        h.redo(&mut st);
        assert!(!h.is_dirty());
    }

    #[test]
    fn saved_state_is_lost_after_diverging() {
        let mut st = Store::new(1);
        let mut h = History::default();
        h.add_stroke(&mut st, line(1.0));
        h.add_stroke(&mut st, line(2.0));
        h.mark_saved(); // saved at len 2
        h.undo(&mut st);
        h.undo(&mut st);
        h.add_stroke(&mut st, line(3.0)); // new branch, len 1
        h.add_stroke(&mut st, line(4.0)); // len 2, but different content
        assert!(h.is_dirty());
    }

    #[test]
    fn one_drag_across_pages_is_one_step() {
        let mut st = Store::new(2);
        let mut h = History::default();
        let mk =
            |page: usize, y: f32| Stroke::new(page, vec![[0.0, y], [10.0, y]], None, 1.0, [0; 3]);
        let strokes = vec![mk(0, 1.0), mk(1, 1.0), mk(0, 2.0), mk(1, 2.0)];
        let ids: Vec<_> = strokes.iter().map(|s| (s.page, s.id)).collect();
        for s in strokes {
            h.add_stroke(&mut st, s);
        }
        let order0: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        let order1: Vec<_> = st.strokes(1).iter().map(|s| s.id).collect();
        // Erase the first stroke of each page, then the second of page 0.
        let mut removed = Vec::new();
        for (page, id) in [ids[0], ids[1], ids[2]] {
            removed.push(st.remove_by_id(page, id).unwrap());
        }
        assert_eq!(h.record_removed(removed), 3);
        assert_eq!(st.total(), 1);
        h.undo(&mut st);
        let back0: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        let back1: Vec<_> = st.strokes(1).iter().map(|s| s.id).collect();
        assert_eq!(back0, order0);
        assert_eq!(back1, order1);
    }

    #[test]
    fn revision_changes_with_every_edit() {
        let mut st = Store::new(1);
        let mut h = History::default();
        let r0 = h.revision();
        h.add_stroke(&mut st, line(1.0));
        let r1 = h.revision();
        h.undo(&mut st);
        let r2 = h.revision();
        h.redo(&mut st);
        assert!(r0 < r1 && r1 < r2 && r2 < h.revision());
    }

    #[test]
    fn removing_nothing_records_nothing() {
        let mut st = Store::new(1);
        let mut h = History::default();
        assert_eq!(h.remove_strokes(&mut st, 0, &[uuid::Uuid::new_v4()]), 0);
        assert!(!h.can_undo());
        assert!(!h.is_dirty());
    }
}
