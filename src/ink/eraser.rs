//! Hit testing for the stroke eraser.

use super::stroke::Stroke;

pub fn dist_point_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a[0] + t * dx, a[1] + t * dy);
    ((p[0] - cx).powi(2) + (p[1] - cy).powi(2)).sqrt()
}

/// Does the eraser circle (page space) touch the stroke?
pub fn hits(stroke: &Stroke, center: [f32; 2], radius: f32) -> bool {
    let b = stroke.bbox;
    if center[0] < b[0] - radius
        || center[0] > b[2] + radius
        || center[1] < b[1] - radius
        || center[1] > b[3] + radius
    {
        return false;
    }
    match stroke.points.len() {
        0 => false,
        1 => {
            let p = stroke.points[0];
            let d = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt();
            d <= radius + stroke.width_at(0) * 0.5
        }
        _ => stroke.points.windows(2).enumerate().any(|(i, w)| {
            let half = stroke.width_at(i).max(stroke.width_at(i + 1)) * 0.5;
            dist_point_segment(center, w[0], w[1]) <= radius + half
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_distance() {
        assert!((dist_point_segment([5.0, 3.0], [0.0, 0.0], [10.0, 0.0]) - 3.0).abs() < 1e-6);
        // Beyond the end: distance to the endpoint.
        assert!((dist_point_segment([13.0, 4.0], [0.0, 0.0], [10.0, 0.0]) - 5.0).abs() < 1e-6);
        // Degenerate segment.
        assert!((dist_point_segment([3.0, 4.0], [0.0, 0.0], [0.0, 0.0]) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn hit_and_miss() {
        let s = Stroke::new(0, vec![[0.0, 0.0], [100.0, 0.0]], None, 2.0, [0; 3]);
        assert!(hits(&s, [50.0, 4.0], 4.0)); // 4 <= 4 + 1
        assert!(hits(&s, [50.0, 5.0], 4.0)); // touches the stroke edge
        assert!(!hits(&s, [50.0, 8.0], 4.0));
        assert!(!hits(&s, [200.0, 0.0], 4.0));
        assert!(hits(&s, [-3.0, 0.0], 4.0));
    }

    #[test]
    fn single_point_stroke() {
        let s = Stroke::new(0, vec![[10.0, 10.0]], None, 2.0, [0; 3]);
        assert!(hits(&s, [12.0, 10.0], 1.5));
        assert!(!hits(&s, [20.0, 10.0], 1.5));
    }

    #[test]
    fn wide_pressure_stroke_uses_local_width() {
        let s = Stroke::new(
            0,
            vec![[0.0, 0.0], [100.0, 0.0]],
            Some(vec![1.0, 1.0]),
            10.0,
            [0; 3],
        );
        assert!(hits(&s, [50.0, 7.0], 3.0)); // 7 <= 3 + 5
    }
}
