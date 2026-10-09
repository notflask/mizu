//! Camera: maps document space (points) to screen space (physical pixels).
//!
//! `screen = (doc - offset) * scale`, where `scale = zoom * dpr` is the
//! number of physical pixels per PDF point.

use super::layout::Layout;

pub const MIN_ZOOM: f32 = 0.1;
pub const MAX_ZOOM: f32 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomMode {
    FitWidth,
    FitPage,
    Free,
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// Document point at the top-left corner of the viewport.
    pub offset: [f32; 2],
    /// Logical pixels per point.
    pub zoom: f32,
    /// Device pixel ratio (physical per logical pixel).
    pub dpr: f32,
    /// Size of the page area in physical pixels.
    pub viewport: [f32; 2],
    pub mode: ZoomMode,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            offset: [0.0, 0.0],
            zoom: 1.0,
            dpr: 1.0,
            viewport: [800.0, 600.0],
            mode: ZoomMode::FitWidth,
        }
    }
}

impl Camera {
    /// Physical pixels per point.
    #[inline]
    pub fn scale(&self) -> f32 {
        self.zoom * self.dpr
    }

    #[inline]
    pub fn doc_to_screen(&self, p: [f32; 2]) -> [f32; 2] {
        let s = self.scale();
        [(p[0] - self.offset[0]) * s, (p[1] - self.offset[1]) * s]
    }

    #[inline]
    pub fn screen_to_doc(&self, p: [f32; 2]) -> [f32; 2] {
        let s = self.scale();
        [p[0] / s + self.offset[0], p[1] / s + self.offset[1]]
    }

    /// Visible document rectangle `[x0, y0, x1, y1]`.
    pub fn visible_doc_rect(&self) -> [f32; 4] {
        let s = self.scale();
        [
            self.offset[0],
            self.offset[1],
            self.offset[0] + self.viewport[0] / s,
            self.offset[1] + self.viewport[1] / s,
        ]
    }

    /// Set a zoom level, keeping the document point under `anchor` (screen,
    /// physical px) fixed.
    pub fn set_zoom_at(&mut self, zoom: f32, anchor: [f32; 2]) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let before = self.screen_to_doc(anchor);
        self.zoom = zoom;
        let s = self.scale();
        self.offset = [before[0] - anchor[0] / s, before[1] - anchor[1] / s];
        self.mode = ZoomMode::Free;
    }

    pub fn zoom_by_at(&mut self, factor: f32, anchor: [f32; 2]) {
        self.set_zoom_at(self.zoom * factor, anchor);
    }

    pub fn viewport_center(&self) -> [f32; 2] {
        [self.viewport[0] * 0.5, self.viewport[1] * 0.5]
    }

    /// Scroll by physical pixels.
    pub fn scroll_px(&mut self, dx: f32, dy: f32) {
        let s = self.scale();
        self.offset[0] += dx / s;
        self.offset[1] += dy / s;
    }

    /// Recompute the zoom for the fit modes. `page` is used by `FitPage`.
    pub fn apply_mode(&mut self, layout: &Layout, page: usize) {
        match self.mode {
            ZoomMode::Free => {}
            ZoomMode::FitWidth => {
                if layout.width > 0.0 {
                    self.zoom = (self.viewport[0] / self.dpr / layout.width).clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }
            ZoomMode::FitPage => {
                if let Some(p) = layout.pages.get(page) {
                    let zw = self.viewport[0] / self.dpr / p.w;
                    let zh = self.viewport[1] / self.dpr / p.h;
                    self.zoom = zw.min(zh).clamp(MIN_ZOOM, MAX_ZOOM);
                }
            }
        }
    }

    /// Keep the view inside the document. Small content is centred.
    pub fn clamp(&mut self, layout: &Layout) {
        let s = self.scale();
        let view_w = self.viewport[0] / s;
        let view_h = self.viewport[1] / s;
        self.offset[0] = clamp_axis(self.offset[0], layout.width, view_w);
        self.offset[1] = clamp_axis(self.offset[1], layout.height, view_h);
    }

    /// Document y of the viewport centre.
    pub fn center_y(&self) -> f32 {
        self.offset[1] + self.viewport[1] / self.scale() * 0.5
    }

    /// Scroll so that document y `y` is at the top of the viewport.
    pub fn scroll_to_y(&mut self, y: f32) {
        self.offset[1] = y;
    }
}

fn clamp_axis(offset: f32, content: f32, view: f32) -> f32 {
    if content <= view {
        (content - view) * 0.5
    } else {
        offset.clamp(0.0, content - view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam() -> Camera {
        Camera {
            offset: [10.0, 500.0],
            zoom: 1.5,
            dpr: 2.0,
            viewport: [1600.0, 900.0],
            mode: ZoomMode::Free,
        }
    }

    #[test]
    fn roundtrip() {
        let c = cam();
        let p = [123.0, 456.0];
        let q = c.screen_to_doc(c.doc_to_screen(p));
        assert!((p[0] - q[0]).abs() < 1e-3 && (p[1] - q[1]).abs() < 1e-3);
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut c = cam();
        let anchor = [400.0, 300.0];
        let before = c.screen_to_doc(anchor);
        for f in [1.1f32, 1.1, 0.5, 3.0, 0.2] {
            c.zoom_by_at(f, anchor);
            let after = c.screen_to_doc(anchor);
            assert!((before[0] - after[0]).abs() < 1e-2, "{before:?} {after:?}");
            assert!((before[1] - after[1]).abs() < 1e-2, "{before:?} {after:?}");
        }
    }

    #[test]
    fn zoom_is_clamped() {
        let mut c = cam();
        c.set_zoom_at(1000.0, [0.0, 0.0]);
        assert_eq!(c.zoom, MAX_ZOOM);
        c.set_zoom_at(0.0001, [0.0, 0.0]);
        assert_eq!(c.zoom, MIN_ZOOM);
    }

    #[test]
    fn fit_width() {
        let layout = Layout::new(&[(600.0, 800.0), (600.0, 800.0)]);
        let mut c = cam();
        c.mode = ZoomMode::FitWidth;
        c.apply_mode(&layout, 0);
        // 1600 physical px / dpr 2 / 600 pt
        assert!((c.zoom - 800.0 / 600.0).abs() < 1e-4);
        assert!((c.zoom * c.dpr * layout.width - c.viewport[0]).abs() < 1e-2);
    }

    #[test]
    fn fit_page_fits_both_axes() {
        let layout = Layout::new(&[(600.0, 800.0)]);
        let mut c = cam();
        c.mode = ZoomMode::FitPage;
        c.apply_mode(&layout, 0);
        assert!(c.zoom * c.dpr * 800.0 <= c.viewport[1] + 1e-2);
        assert!(c.zoom * c.dpr * 600.0 <= c.viewport[0] + 1e-2);
    }

    #[test]
    fn clamp_limits_and_centres() {
        let layout = Layout::new(&[(600.0, 800.0), (600.0, 800.0)]);
        let mut c = cam();
        c.offset = [-1000.0, -1000.0];
        c.clamp(&layout);
        assert!(c.offset[1] >= 0.0);
        c.offset = [1000.0, 1.0e6];
        c.clamp(&layout);
        let view_h = c.viewport[1] / c.scale();
        assert!((c.offset[1] - (layout.height - view_h)).abs() < 1e-3);

        // Narrow content is centred horizontally.
        let narrow = Layout::new(&[(100.0, 100.0)]);
        c.clamp(&narrow);
        let view_w = c.viewport[0] / c.scale();
        assert!((c.offset[0] - (100.0 - view_w) * 0.5).abs() < 1e-3);
    }
}
