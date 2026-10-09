//! Pen and eraser.

use super::{Tool, UiMode, Viewer};
use crate::ink::smooth::{self, PressureFilter};
use crate::ink::Stroke;
use crate::render::{LiveStroke, PenCursor};

/// Minimum distance between recorded points, in physical pixels.
const MIN_STEP_PX: f32 = 0.75;
/// Eraser radius in logical pixels.
const ERASER_RADIUS: f32 = 10.0;

enum Active {
    Draw {
        page: usize,
        points: Vec<[f32; 2]>,
        pressure: Option<Vec<f32>>,
        filter: PressureFilter,
    },
    Erase {
        removed: Vec<(usize, Stroke)>,
        last: Option<(usize, [f32; 2])>,
    },
}

#[derive(Default)]
pub struct PenState {
    active: Option<Active>,
}

impl PenState {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub fn is_erasing(&self) -> bool {
        matches!(self.active, Some(Active::Erase { .. }))
    }

    /// Abort whatever is in progress. Erased strokes are *not* restored
    /// here; callers that need that use `Viewer::pen_up`.
    pub fn cancel(&mut self) {
        self.active = None;
    }
}

impl Viewer {
    pub fn live_stroke(&self) -> Option<LiveStroke<'_>> {
        match &self.pen.active {
            Some(Active::Draw {
                page,
                points,
                pressure,
                ..
            }) => Some(LiveStroke {
                page: *page,
                points,
                pressure: pressure.as_deref(),
                width: self.pen_width,
                color: self.pen_color,
            }),
            _ => None,
        }
    }

    pub fn pen_cursor(&self) -> Option<PenCursor> {
        if self.mode != UiMode::Draw || !self.mouse.inside {
            return None;
        }
        let erasing = self.tool == Tool::Eraser || self.pen.is_erasing();
        let radius = if erasing {
            ERASER_RADIUS * self.camera.dpr
        } else {
            (self.pen_width * self.camera.scale() * 0.5).max(1.5)
        };
        Some(PenCursor {
            pos: self.mouse.pos,
            radius,
            color: self.pen_color,
            eraser: erasing,
        })
    }

    /// Start a stroke (or an erase drag). `pressure` is `None` for a mouse.
    pub fn pen_down(&mut self, screen: [f32; 2], pressure: Option<f32>, erase: bool) {
        if self.doc.is_none() || self.pen.is_active() {
            return;
        }
        if erase {
            self.pen.active = Some(Active::Erase {
                removed: Vec::new(),
                last: None,
            });
            self.erase_at(screen);
            return;
        }
        let Some((page, p)) = self.locate(screen, false) else {
            return;
        };
        let mut filter = PressureFilter::new();
        let pressure = pressure.map(|v| vec![filter.push(v)]);
        self.pen.active = Some(Active::Draw {
            page,
            points: vec![p],
            pressure,
            filter,
        });
        self.dirty = true;
    }

    pub fn pen_move(&mut self, screen: [f32; 2], pressure: Option<f32>) {
        let scale = self.camera.scale();
        if matches!(self.pen.active, Some(Active::Erase { .. })) {
            self.erase_at(screen);
            return;
        }
        let Some(Active::Draw { page, .. }) = &self.pen.active else {
            return;
        };
        let page = *page;
        let Some(d) = &self.doc else { return };
        let Some(g) = d.layout.pages.get(page) else {
            return;
        };
        let doc = self.camera.screen_to_doc(screen);
        let p = [
            (doc[0] - g.x).clamp(0.0, g.w),
            (doc[1] - g.y).clamp(0.0, g.h),
        ];
        if let Some(Active::Draw {
            points,
            pressure: pr,
            filter,
            ..
        }) = &mut self.pen.active
        {
            if let Some(last) = points.last() {
                let dist = ((p[0] - last[0]).powi(2) + (p[1] - last[1]).powi(2)).sqrt() * scale;
                if dist < MIN_STEP_PX {
                    return;
                }
            }
            points.push(p);
            match (pr.as_mut(), pressure) {
                (Some(v), Some(x)) => v.push(filter.push(x)),
                (Some(v), None) => {
                    let last = v.last().copied().unwrap_or(1.0);
                    v.push(last);
                }
                _ => {}
            }
        }
        self.dirty = true;
    }

    pub fn pen_up(&mut self) {
        let Some(active) = self.pen.active.take() else {
            return;
        };
        match active {
            Active::Draw {
                page,
                mut points,
                mut pressure,
                ..
            } => {
                if points.len() >= 3 {
                    smooth::finish(&mut points, &mut pressure, 0.1);
                }
                let stroke = Stroke::new(page, points, pressure, self.pen_width, self.pen_color);
                if let Some(d) = &mut self.doc {
                    d.history.add_stroke(&mut d.ink, stroke);
                }
            }
            Active::Erase { removed, .. } => {
                if let Some(d) = &mut self.doc {
                    d.history.record_removed(removed);
                }
            }
        }
        self.dirty = true;
    }

    fn erase_at(&mut self, screen: [f32; 2]) {
        let scale = self.camera.scale();
        let radius = ERASER_RADIUS * self.camera.dpr / scale;
        let Some((page, p)) = self.locate(screen, false) else {
            if let Some(Active::Erase { last, .. }) = &mut self.pen.active {
                *last = None;
            }
            return;
        };
        // Sample along the way so fast drags do not skip thin strokes.
        let mut samples = vec![p];
        if let Some(Active::Erase {
            last: Some((lp, lpos)),
            ..
        }) = &self.pen.active
        {
            if *lp == page {
                let dist = ((p[0] - lpos[0]).powi(2) + (p[1] - lpos[1]).powi(2)).sqrt();
                let n = (dist / (radius * 0.5)).ceil() as usize;
                for i in 1..n.min(200) {
                    let t = i as f32 / n as f32;
                    samples.push([
                        lpos[0] + (p[0] - lpos[0]) * t,
                        lpos[1] + (p[1] - lpos[1]) * t,
                    ]);
                }
            }
        }
        let Some(d) = &mut self.doc else { return };
        let mut got: Vec<(usize, Stroke)> = Vec::new();
        for s in samples {
            for id in d.ink.hit_test(page, s, radius) {
                if let Some(r) = d.ink.remove_by_id(page, id) {
                    got.push(r);
                }
            }
        }
        if let Some(Active::Erase { removed, last }) = &mut self.pen.active {
            removed.extend(got);
            *last = Some((page, p));
        }
        self.dirty = true;
    }
}
