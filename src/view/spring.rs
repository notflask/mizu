//! Scroll animation: a critically damped spring.
//!
//! Unlike a plain exponential ease, the spring keeps its velocity when the
//! target moves (wheel notches in a row, held `j`), so motion stays continuous
//! instead of restarting the curve on every event. It also starts gently,
//! which hides the uneven timing of the first frame after the window was idle.

/// Stiffness (1/s). A single wheel notch settles in about 150 ms.
pub const OMEGA: f32 = 44.0;

/// Longest step the animation takes in one frame. Longer gaps (the window
/// was idle, a frame was late) are treated as one frame at 30 Hz.
pub const MAX_DT: f32 = 1.0 / 30.0;

/// Step assumed for the first frame of an animation that starts from rest,
/// because the time since the previous frame says nothing about it.
pub const FIRST_DT: f32 = 1.0 / 60.0;

/// Advance `pos`/`vel` towards `target` by `dt` seconds (exact solution of
/// the critically damped oscillator, so it is stable for any `dt`).
pub fn step(pos: f32, vel: f32, target: f32, dt: f32, omega: f32) -> (f32, f32) {
    let e0 = pos - target;
    let c = vel + omega * e0;
    let decay = (-omega * dt).exp();
    let e = (e0 + c * dt) * decay;
    let v = (vel - omega * c * dt) * decay;
    (target + e, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(from: f32, to: f32, hz: f32, frames: usize) -> Vec<f32> {
        let (mut p, mut v) = (from, 0.0);
        let mut out = Vec::new();
        for _ in 0..frames {
            (p, v) = step(p, v, to, 1.0 / hz, OMEGA);
            out.push(p);
        }
        out
    }

    #[test]
    fn first_frame_moves_only_a_little() {
        let p = run(0.0, 100.0, 60.0, 1);
        assert!(p[0] > 0.0 && p[0] < 25.0, "first step {}", p[0]);
    }

    #[test]
    fn converges_without_overshoot() {
        let p = run(0.0, 100.0, 60.0, 12);
        assert!(p.windows(2).all(|w| w[1] >= w[0]));
        assert!(p.iter().all(|&x| x <= 100.0 + 1e-3));
        // ~200 ms later it is there.
        assert!((p[11] - 100.0).abs() < 0.5, "after 12 frames: {}", p[11]);
    }

    #[test]
    fn retargeting_keeps_velocity() {
        let (mut p, mut v) = (0.0, 0.0);
        for _ in 0..3 {
            (p, v) = step(p, v, 100.0, 1.0 / 60.0, OMEGA);
        }
        let v_before = v;
        // A second notch: the target moves on, motion continues forward.
        let (p2, v2) = step(p, v, 200.0, 1.0 / 60.0, OMEGA);
        assert!(p2 > p);
        assert!(v2 >= v_before * 0.9, "{v2} vs {v_before}");
    }

    #[test]
    fn same_motion_at_any_refresh_rate() {
        let a = run(0.0, 100.0, 60.0, 6)[5];
        let b = run(0.0, 100.0, 120.0, 12)[11];
        assert!((a - b).abs() < 0.01);
    }
}
