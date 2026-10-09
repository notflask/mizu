//! One pen stroke, stored in page space (PDF points, y down).

use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub id: Uuid,
    pub page: usize,
    pub points: Vec<[f32; 2]>,
    /// Pressure per point in `0..=1`, if the stroke came from a pen.
    pub pressure: Option<Vec<f32>>,
    /// Base width in points.
    pub width: f32,
    pub color: [u8; 3],
    /// `[x0, y0, x1, y1]`, padded by the widest half-width.
    pub bbox: [f32; 4],
}

/// Width of a point drawn with pressure `p` (0..=1).
pub fn pressure_width(base: f32, p: f32) -> f32 {
    base * (0.25 + 0.75 * p.clamp(0.0, 1.0).powf(0.75))
}

impl Stroke {
    pub fn new(
        page: usize,
        points: Vec<[f32; 2]>,
        pressure: Option<Vec<f32>>,
        width: f32,
        color: [u8; 3],
    ) -> Stroke {
        let mut s = Stroke {
            id: Uuid::new_v4(),
            page,
            points,
            pressure,
            width,
            color,
            bbox: [0.0; 4],
        };
        s.update_bbox();
        s
    }

    /// Full width at point `i`.
    pub fn width_at(&self, i: usize) -> f32 {
        match &self.pressure {
            Some(p) => pressure_width(self.width, p.get(i).copied().unwrap_or(1.0)),
            None => self.width,
        }
    }

    pub fn max_width(&self) -> f32 {
        match &self.pressure {
            Some(p) => p
                .iter()
                .fold(0.0f32, |m, &v| m.max(pressure_width(self.width, v)))
                .max(self.width * 0.25),
            None => self.width,
        }
    }

    /// Average width, written to `/BS /W` when saving.
    pub fn mean_width(&self) -> f32 {
        match &self.pressure {
            Some(p) if !p.is_empty() => {
                p.iter()
                    .map(|&v| pressure_width(self.width, v))
                    .sum::<f32>()
                    / p.len() as f32
            }
            _ => self.width,
        }
    }

    pub fn update_bbox(&mut self) {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &self.points {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        if self.points.is_empty() {
            b = [0.0; 4];
        }
        let pad = self.max_width() * 0.5;
        self.bbox = [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_is_padded_by_half_width() {
        let s = Stroke::new(0, vec![[0.0, 0.0], [10.0, 4.0]], None, 2.0, [0; 3]);
        assert_eq!(s.bbox, [-1.0, -1.0, 11.0, 5.0]);
    }

    #[test]
    fn pressure_scales_width() {
        assert!((pressure_width(2.0, 1.0) - 2.0).abs() < 1e-6);
        assert!((pressure_width(2.0, 0.0) - 0.5).abs() < 1e-6);
        assert!(pressure_width(2.0, 0.5) > pressure_width(2.0, 0.25));
        let s = Stroke::new(
            0,
            vec![[0.0, 0.0], [1.0, 0.0]],
            Some(vec![0.0, 1.0]),
            2.0,
            [0; 3],
        );
        assert!((s.width_at(0) - 0.5).abs() < 1e-6);
        assert!((s.width_at(1) - 2.0).abs() < 1e-6);
        assert!((s.mean_width() - 1.25).abs() < 1e-6);
    }
}
