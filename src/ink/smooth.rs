//! Cleaning up raw mouse / pen input once a stroke is finished.

/// Online exponential smoothing for pen pressure.
#[derive(Clone, Copy, Debug)]
pub struct PressureFilter {
    value: Option<f32>,
}

impl PressureFilter {
    pub const ALPHA: f32 = 0.4;

    pub fn new() -> Self {
        PressureFilter { value: None }
    }

    pub fn push(&mut self, p: f32) -> f32 {
        let p = p.clamp(0.0, 1.0);
        let v = match self.value {
            None => p,
            Some(prev) => prev + Self::ALPHA * (p - prev),
        };
        self.value = Some(v);
        v
    }
}

impl Default for PressureFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Weighted 1-2-1 moving average, `passes` times. Endpoints are kept.
pub fn smooth_points(points: &mut [[f32; 2]], passes: usize) {
    let n = points.len();
    if n < 3 {
        return;
    }
    let mut tmp = points.to_vec();
    for _ in 0..passes {
        for i in 1..n - 1 {
            for k in 0..2 {
                tmp[i][k] = 0.25 * points[i - 1][k] + 0.5 * points[i][k] + 0.25 * points[i + 1][k];
            }
        }
        points.copy_from_slice(&tmp);
    }
}

pub fn smooth_scalars(values: &mut [f32], passes: usize) {
    let n = values.len();
    if n < 3 {
        return;
    }
    let mut tmp = values.to_vec();
    for _ in 0..passes {
        for i in 1..n - 1 {
            tmp[i] = 0.25 * values[i - 1] + 0.5 * values[i] + 0.25 * values[i + 1];
        }
        values.copy_from_slice(&tmp);
    }
}

fn dist_to_chord(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    if len2 <= f32::EPSILON {
        return ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt();
    }
    ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len2.sqrt()
}

/// Ramer–Douglas–Peucker. Returns the indices of the points to keep
/// (always including the first and the last).
pub fn rdp_indices(points: &[[f32; 2]], eps: f32) -> Vec<usize> {
    let n = points.len();
    if n <= 2 {
        return (0..n).collect();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (mut max_d, mut max_i) = (0.0f32, lo);
        for i in lo + 1..hi {
            let d = dist_to_chord(points[i], points[lo], points[hi]);
            if d > max_d {
                max_d = d;
                max_i = i;
            }
        }
        if max_d > eps {
            keep[max_i] = true;
            stack.push((lo, max_i));
            stack.push((max_i, hi));
        }
    }
    keep.iter()
        .enumerate()
        .filter_map(|(i, &k)| k.then_some(i))
        .collect()
}

/// Smooth, then simplify a finished stroke in place.
pub fn finish(points: &mut Vec<[f32; 2]>, pressure: &mut Option<Vec<f32>>, eps: f32) {
    smooth_points(points, 2);
    if let Some(p) = pressure.as_mut() {
        smooth_scalars(p, 2);
    }
    let keep = rdp_indices(points, eps);
    if keep.len() < points.len() {
        let new_points: Vec<[f32; 2]> = keep.iter().map(|&i| points[i]).collect();
        if let Some(p) = pressure.as_mut() {
            let new_p: Vec<f32> = keep.iter().map(|&i| p[i]).collect();
            *p = new_p;
        }
        *points = new_points;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoothing_keeps_endpoints() {
        let mut pts = vec![[0.0, 0.0], [1.0, 5.0], [2.0, -5.0], [3.0, 5.0], [4.0, 0.0]];
        let first = pts[0];
        let last = *pts.last().unwrap();
        smooth_points(&mut pts, 2);
        assert_eq!(pts[0], first);
        assert_eq!(*pts.last().unwrap(), last);
        // Peaks are reduced.
        assert!(pts[1][1] < 5.0);
        assert!(pts[2][1].abs() < 5.0);
    }

    #[test]
    fn smoothing_short_strokes_is_noop() {
        let mut pts = vec![[0.0, 0.0], [1.0, 1.0]];
        smooth_points(&mut pts, 3);
        assert_eq!(pts, vec![[0.0, 0.0], [1.0, 1.0]]);
    }

    #[test]
    fn rdp_removes_collinear_points() {
        let pts: Vec<[f32; 2]> = (0..50).map(|i| [i as f32, 2.0 * i as f32]).collect();
        let keep = rdp_indices(&pts, 0.1);
        assert_eq!(keep, vec![0, 49]);
    }

    #[test]
    fn rdp_keeps_corners() {
        let mut pts: Vec<[f32; 2]> = (0..=10).map(|i| [i as f32, 0.0]).collect();
        pts.extend((1..=10).map(|i| [10.0, i as f32]));
        let keep = rdp_indices(&pts, 0.1);
        assert_eq!(keep.len(), 3);
        assert!(keep.contains(&10));
    }

    #[test]
    fn rdp_handles_degenerate_input() {
        assert_eq!(rdp_indices(&[], 0.1), Vec::<usize>::new());
        assert_eq!(rdp_indices(&[[1.0, 1.0]], 0.1), vec![0]);
        // Closed loop: first == last must not divide by zero.
        let pts = vec![[0.0, 0.0], [5.0, 5.0], [0.0, 0.0]];
        let keep = rdp_indices(&pts, 0.1);
        assert_eq!(keep, vec![0, 1, 2]);
    }

    #[test]
    fn finish_keeps_pressure_aligned() {
        let mut pts: Vec<[f32; 2]> = (0..30).map(|i| [i as f32, 0.0]).collect();
        let mut pr = Some((0..30).map(|i| i as f32 / 29.0).collect::<Vec<_>>());
        finish(&mut pts, &mut pr, 0.1);
        assert_eq!(pts.len(), pr.as_ref().unwrap().len());
        assert_eq!(pts.len(), 2);
        let p = pr.unwrap();
        assert!(p[0] < 0.01 && p[1] > 0.99);
    }

    #[test]
    fn pressure_filter_converges() {
        let mut f = PressureFilter::new();
        assert_eq!(f.push(0.5), 0.5);
        let a = f.push(1.0);
        assert!(a > 0.5 && a < 1.0);
        let mut v = a;
        for _ in 0..50 {
            v = f.push(1.0);
        }
        assert!(v > 0.99);
        assert_eq!(PressureFilter::new().push(7.0), 1.0);
    }
}
