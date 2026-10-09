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
}

impl Default for History {
    fn default() -> Self {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            saved_len: Some(0),
        }
    }
}

impl History {
    pub fn is_dirty(&self) -> bool {
        self.saved_len != Some(self.undo.len())
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
        if removed.is_empty() {
            return 0;
        }
        // `removed` holds indices relative to the state after the previous
        // removals. Convert each to the state before the first removal so
        // that undo can re-insert in ascending order.
        let n = removed.len();
        let raw: Vec<usize> = removed.iter().map(|(i, _)| *i).collect();
        let mut orig: Vec<(usize, Stroke)> = Vec::with_capacity(n);
        for (i, (_, s)) in removed.into_iter().enumerate() {
            let mut x = raw[i];
            for j in (0..i).rev() {
                if x >= raw[j] {
                    x += 1;
                }
            }
            orig.push((x, s));
        }
        orig.sort_by_key(|(i, _)| *i);
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
        assert!(!st.strokes(0).iter().any(|s| s.id == order[1] || s.id == order[3]));
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
    fn removing_nothing_records_nothing() {
        let mut st = Store::new(1);
        let mut h = History::default();
        assert_eq!(h.remove_strokes(&mut st, 0, &[uuid::Uuid::new_v4()]), 0);
        assert!(!h.can_undo());
        assert!(!h.is_dirty());
    }
}
