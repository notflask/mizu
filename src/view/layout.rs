//! Vertical page layout in document space (PDF points, y down).

use std::ops::Range;

/// Gap between two pages, in points.
pub const PAGE_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeom {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PageGeom {
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pages: Vec<PageGeom>,
    pub width: f32,
    pub height: f32,
}

impl Layout {
    /// Stack pages vertically, centred on the widest one.
    pub fn new(sizes: &[(f32, f32)]) -> Self {
        let width = sizes.iter().fold(0.0f32, |m, s| m.max(s.0));
        let mut y = 0.0;
        let mut pages = Vec::with_capacity(sizes.len());
        for &(w, h) in sizes {
            pages.push(PageGeom {
                x: (width - w) * 0.5,
                y,
                w,
                h,
            });
            y += h + PAGE_GAP;
        }
        let height = if pages.is_empty() { 0.0 } else { y - PAGE_GAP };
        Layout {
            pages,
            width,
            height,
        }
    }

    pub fn len(&self) -> usize {
        self.pages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Index of the page containing `y`, or the nearest one when `y` falls
    /// into a gap or outside the document.
    pub fn page_at_y(&self, y: f32) -> usize {
        if self.pages.is_empty() {
            return 0;
        }
        // First page whose bottom is below y.
        let idx = self
            .pages
            .partition_point(|p| p.bottom() + PAGE_GAP * 0.5 <= y);
        idx.min(self.pages.len() - 1)
    }

    /// Pages intersecting the vertical span `[y0, y1]`.
    pub fn visible(&self, y0: f32, y1: f32) -> Range<usize> {
        if self.pages.is_empty() {
            return 0..0;
        }
        let first = self.pages.partition_point(|p| p.bottom() < y0);
        let last = self.pages.partition_point(|p| p.y <= y1);
        first.min(self.pages.len())..last.max(first).min(self.pages.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Layout {
        Layout::new(&[(100.0, 200.0), (200.0, 100.0), (100.0, 100.0)])
    }

    #[test]
    fn stacks_and_centres() {
        let l = sample();
        assert_eq!(l.width, 200.0);
        assert_eq!(l.pages[0].x, 50.0);
        assert_eq!(l.pages[1].x, 0.0);
        assert_eq!(l.pages[1].y, 200.0 + PAGE_GAP);
        assert_eq!(l.height, 200.0 + 100.0 + 100.0 + 2.0 * PAGE_GAP);
    }

    #[test]
    fn page_lookup() {
        let l = sample();
        assert_eq!(l.page_at_y(-50.0), 0);
        assert_eq!(l.page_at_y(10.0), 0);
        assert_eq!(l.page_at_y(l.pages[1].y + 1.0), 1);
        assert_eq!(l.page_at_y(l.pages[2].y + 1.0), 2);
        assert_eq!(l.page_at_y(1.0e6), 2);
    }

    #[test]
    fn visible_range() {
        let l = sample();
        assert_eq!(l.visible(0.0, 10.0), 0..1);
        assert_eq!(l.visible(190.0, 220.0), 0..2);
        assert_eq!(l.visible(0.0, 1.0e6), 0..3);
        assert_eq!(l.visible(l.pages[2].y, l.pages[2].y + 1.0), 2..3);
    }

    #[test]
    fn empty_layout() {
        let l = Layout::new(&[]);
        assert!(l.is_empty());
        assert_eq!(l.visible(0.0, 100.0), 0..0);
        assert_eq!(l.page_at_y(5.0), 0);
    }
}
