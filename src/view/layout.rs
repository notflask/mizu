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
    /// For every page, the first page of its row (itself without spreads).
    row_start: Vec<usize>,
}

/// Where a page wants to sit in a two-page spread (from the book).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Auto,
    Left,
    Right,
    Alone,
}

impl Layout {
    /// Stack pages vertically, centred on the widest one.
    pub fn new(sizes: &[(f32, f32)]) -> Self {
        let rows: Vec<Range<usize>> = (0..sizes.len()).map(|i| i..i + 1).collect();
        Self::with_rows(sizes, &rows, false)
    }

    /// Two pages side by side, like an open book. `rtl` puts the first page
    /// of a pair on the right (manga). The cover and wide pages stand alone.
    pub fn spreads(sizes: &[(f32, f32)], sides: &[Side], rtl: bool) -> Self {
        Self::with_rows(sizes, &spread_rows(sizes, sides, rtl), rtl)
    }

    fn with_rows(sizes: &[(f32, f32)], rows: &[Range<usize>], rtl: bool) -> Self {
        let row_w = |r: &Range<usize>| sizes[r.clone()].iter().map(|s| s.0).sum::<f32>();
        let width = rows.iter().map(row_w).fold(0.0f32, f32::max);
        let mut y = 0.0;
        let mut pages = vec![
            PageGeom {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0
            };
            sizes.len()
        ];
        let mut row_start = vec![0; sizes.len()];
        for r in rows {
            let rw = row_w(r);
            let rh = sizes[r.clone()].iter().map(|s| s.1).fold(0.0f32, f32::max);
            // Pages of a spread touch; the pair is centred as a whole.
            let mut x = (width - rw) * 0.5;
            let order: Vec<usize> = if rtl {
                r.clone().rev().collect()
            } else {
                r.clone().collect()
            };
            for i in order {
                let (w, h) = sizes[i];
                pages[i] = PageGeom {
                    x,
                    y: y + (rh - h) * 0.5,
                    w,
                    h,
                };
                row_start[i] = r.start;
                x += w;
            }
            y += rh + PAGE_GAP;
        }
        let height = if pages.is_empty() { 0.0 } else { y - PAGE_GAP };
        Layout {
            pages,
            width,
            height,
            row_start,
        }
    }

    /// First page of the row after the one `page` is in.
    pub fn next_row(&self, page: usize) -> usize {
        let start = self.row_start.get(page).copied().unwrap_or(page);
        // The last row has no next one: stay.
        (start..self.pages.len())
            .find(|&i| self.row_start[i] != start)
            .unwrap_or(start)
    }

    /// First page of the row before the one `page` is in.
    pub fn prev_row(&self, page: usize) -> usize {
        let start = self.row_start.get(page).copied().unwrap_or(page);
        if start == 0 {
            return 0;
        }
        self.row_start[start - 1]
    }

    /// First page of the row `page` is in.
    pub fn row_of(&self, page: usize) -> usize {
        self.row_start.get(page).copied().unwrap_or(page)
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

/// Group pages into spread rows (each a range of consecutive pages).
pub fn spread_rows(sizes: &[(f32, f32)], sides: &[Side], rtl: bool) -> Vec<Range<usize>> {
    let side = |i: usize| sides.get(i).copied().unwrap_or_default();
    let alone = |i: usize| {
        let (w, h) = sizes[i];
        w > h || side(i) == Side::Alone
    };
    // The side the first page of a pair sits on, and the other one.
    let (lead, trail) = if rtl {
        (Side::Right, Side::Left)
    } else {
        (Side::Left, Side::Right)
    };
    let mut rows = Vec::new();
    let mut i = 0;
    while i < sizes.len() {
        let can_pair = !alone(i)
            && side(i) != trail
            // The cover stands alone unless the book pairs it explicitly.
            && (i > 0 || side(0) == lead)
            && i + 1 < sizes.len()
            && !alone(i + 1)
            && side(i + 1) != lead;
        if can_pair {
            rows.push(i..i + 2);
            i += 2;
        } else {
            rows.push(i..i + 1);
            i += 1;
        }
    }
    rows
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

    #[test]
    fn spreads_pair_pages_with_the_cover_alone() {
        let sizes = [(100.0, 150.0); 6];
        let rows = spread_rows(&sizes, &[], true);
        assert_eq!(rows, vec![0..1, 1..3, 3..5, 5..6]);
        // A wide (double) page stands alone and does not break the rhythm.
        let mut sizes = vec![(100.0, 150.0); 5];
        sizes[2] = (200.0, 150.0);
        let rows = spread_rows(&sizes, &[], false);
        assert_eq!(rows, vec![0..1, 1..2, 2..3, 3..5]);
        // Explicit sides: a left page can not start an RTL pair.
        let sides = [Side::Auto, Side::Left, Side::Right, Side::Left];
        let rows = spread_rows(&[(100.0, 150.0); 4], &sides, true);
        assert_eq!(rows, vec![0..1, 1..2, 2..4]);
    }

    #[test]
    fn rtl_puts_the_first_page_on_the_right() {
        let l = Layout::spreads(&[(100.0, 150.0); 3], &[], true);
        assert_eq!(l.width, 200.0);
        assert_eq!(l.pages[0].x, 50.0); // the cover, centred
        assert!(l.pages[1].x > l.pages[2].x);
        assert_eq!(l.pages[1].y, l.pages[2].y);
        let l = Layout::spreads(&[(100.0, 150.0); 3], &[], false);
        assert!(l.pages[1].x < l.pages[2].x);
        assert_eq!(l.next_row(0), 1);
        assert_eq!(l.next_row(1), 1); // pages 1..3 are the last row
        assert_eq!(l.row_of(2), 1);
        assert_eq!(l.prev_row(2), 0);
        // Visible / page_at_y keep working.
        assert_eq!(l.page_at_y(200.0), 1);
        assert_eq!(l.visible(0.0, 400.0), 0..3);
    }
}
