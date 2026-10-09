//! All strokes of a document, grouped by page, with a coarse spatial grid
//! per page so that erasing stays fast with thousands of strokes.

use std::collections::HashMap;

use super::eraser;
use super::stroke::Stroke;

/// Grid cell edge length in points.
const CELL: f32 = 32.0;

#[derive(Default)]
struct PageInk {
    strokes: Vec<Stroke>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    grid_dirty: bool,
}

impl PageInk {
    fn rebuild_grid(&mut self) {
        self.grid.clear();
        for (i, s) in self.strokes.iter().enumerate() {
            let (x0, y0, x1, y1) = cell_range(s.bbox);
            for cx in x0..=x1 {
                for cy in y0..=y1 {
                    self.grid.entry((cx, cy)).or_default().push(i as u32);
                }
            }
        }
        self.grid_dirty = false;
    }
}

fn cell_range(b: [f32; 4]) -> (i32, i32, i32, i32) {
    (
        (b[0] / CELL).floor() as i32,
        (b[1] / CELL).floor() as i32,
        (b[2] / CELL).floor() as i32,
        (b[3] / CELL).floor() as i32,
    )
}

#[derive(Default)]
pub struct Store {
    pages: Vec<PageInk>,
    /// Bumped on every change of a page, so renderers know when to re-upload.
    generations: Vec<u64>,
    counter: u64,
}

impl Store {
    pub fn new(page_count: usize) -> Self {
        Store {
            pages: (0..page_count).map(|_| PageInk::default()).collect(),
            generations: vec![0; page_count],
            counter: 0,
        }
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    fn touch(&mut self, page: usize) {
        self.counter += 1;
        self.generations[page] = self.counter;
        self.pages[page].grid_dirty = true;
    }

    pub fn generation(&self, page: usize) -> u64 {
        self.generations.get(page).copied().unwrap_or(0)
    }

    pub fn strokes(&self, page: usize) -> &[Stroke] {
        self.pages
            .get(page)
            .map(|p| p.strokes.as_slice())
            .unwrap_or(&[])
    }

    pub fn total(&self) -> usize {
        self.pages.iter().map(|p| p.strokes.len()).sum()
    }

    pub fn all(&self) -> impl Iterator<Item = &Stroke> {
        self.pages.iter().flat_map(|p| p.strokes.iter())
    }

    pub fn push(&mut self, stroke: Stroke) {
        let page = stroke.page;
        if page >= self.pages.len() {
            return;
        }
        self.pages[page].strokes.push(stroke);
        self.touch(page);
    }

    /// Insert at a specific index (used by undo to restore the draw order).
    pub fn insert(&mut self, index: usize, stroke: Stroke) {
        let page = stroke.page;
        if page >= self.pages.len() {
            return;
        }
        let v = &mut self.pages[page].strokes;
        let at = index.min(v.len());
        v.insert(at, stroke);
        self.touch(page);
    }

    /// Remove the stroke with `id` on `page`; returns it with its index.
    pub fn remove_by_id(&mut self, page: usize, id: uuid::Uuid) -> Option<(usize, Stroke)> {
        let v = &mut self.pages.get_mut(page)?.strokes;
        let idx = v.iter().position(|s| s.id == id)?;
        let s = v.remove(idx);
        self.touch(page);
        Some((idx, s))
    }

    pub fn pop_last(&mut self, page: usize) -> Option<Stroke> {
        let s = self.pages.get_mut(page)?.strokes.pop()?;
        self.touch(page);
        Some(s)
    }

    /// Indices of strokes on `page` touched by the eraser circle.
    pub fn hit_test(&mut self, page: usize, center: [f32; 2], radius: f32) -> Vec<uuid::Uuid> {
        let Some(p) = self.pages.get_mut(page) else {
            return Vec::new();
        };
        if p.strokes.is_empty() {
            return Vec::new();
        }
        if p.grid_dirty {
            p.rebuild_grid();
        }
        let area = [
            center[0] - radius,
            center[1] - radius,
            center[0] + radius,
            center[1] + radius,
        ];
        let (x0, y0, x1, y1) = cell_range(area);
        let mut seen: Vec<u32> = Vec::new();
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                if let Some(list) = p.grid.get(&(cx, cy)) {
                    seen.extend_from_slice(list);
                }
            }
        }
        seen.sort_unstable();
        seen.dedup();
        seen.into_iter()
            .filter_map(|i| {
                let s = &p.strokes[i as usize];
                eraser::hits(s, center, radius).then_some(s.id)
            })
            .collect()
    }

    pub fn clear(&mut self) {
        let n = self.pages.len();
        for p in 0..n {
            if !self.pages[p].strokes.is_empty() {
                self.pages[p].strokes.clear();
                self.touch(p);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(page: usize, y: f32) -> Stroke {
        Stroke::new(page, vec![[0.0, y], [100.0, y]], None, 1.0, [0; 3])
    }

    #[test]
    fn push_remove_insert_keep_order() {
        let mut st = Store::new(2);
        let a = line(0, 10.0);
        let b = line(0, 50.0);
        let c = line(0, 90.0);
        let (ida, idb, idc) = (a.id, b.id, c.id);
        st.push(a);
        st.push(b);
        st.push(c);
        let (idx, removed) = st.remove_by_id(0, idb).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(st.strokes(0).len(), 2);
        st.insert(idx, removed);
        let ids: Vec<_> = st.strokes(0).iter().map(|s| s.id).collect();
        assert_eq!(ids, vec![ida, idb, idc]);
    }

    #[test]
    fn generations_change_on_edit() {
        let mut st = Store::new(2);
        let g0 = st.generation(0);
        st.push(line(0, 1.0));
        assert!(st.generation(0) > g0);
        assert_eq!(st.generation(1), 0);
    }

    #[test]
    fn hit_test_finds_only_nearby_strokes() {
        let mut st = Store::new(1);
        let near = line(0, 100.0);
        let far = line(0, 400.0);
        let near_id = near.id;
        st.push(near);
        st.push(far);
        let hits = st.hit_test(0, [50.0, 102.0], 4.0);
        assert_eq!(hits, vec![near_id]);
        assert!(st.hit_test(0, [50.0, 250.0], 4.0).is_empty());
        // Grid is rebuilt after removal.
        st.remove_by_id(0, near_id);
        assert!(st.hit_test(0, [50.0, 102.0], 4.0).is_empty());
    }

    #[test]
    fn many_strokes_hit_test_is_exact() {
        let mut st = Store::new(1);
        let mut target = None;
        for i in 0..2000 {
            let s = line(0, i as f32 * 3.0);
            if i == 1000 {
                target = Some(s.id);
            }
            st.push(s);
        }
        let hits = st.hit_test(0, [50.0, 3000.0], 0.5);
        assert_eq!(hits, vec![target.unwrap()]);
    }

    #[test]
    fn out_of_range_pages_are_ignored() {
        let mut st = Store::new(1);
        st.push(line(5, 1.0));
        assert_eq!(st.total(), 0);
        assert!(st.hit_test(9, [0.0, 0.0], 1.0).is_empty());
    }
}
