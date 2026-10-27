//! Motion: the damped-spring curve every iris surface shares.
//!
//! Closed form, k=280, c=30, zeta ~0.9 (DESIGN.md). Normalized time
//! 0..1 maps to a ~0.55s settle. The raw curve overshoots past 1
//! briefly; the output is clamped to 0..=1 because GPUI's animation
//! driver panics on deltas outside that range, and callers drive it
//! manually per frame.

use std::time::Duration;

/// The shared timing vocabulary. Every duration a surface animates
/// over comes from here so the app accelerates and settles as one.
pub const FADE: Duration = Duration::from_millis(140);
pub const ENTER: Duration = Duration::from_millis(380);

/// Quadratic ease-out: the default curve for fades and slides.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Cubic ease-out: larger travel (menus, sheets).
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// QA harness: IRIS_SLOWMO=4 makes every animation run 4x slower so
/// curves can be verified frame by frame. Absent or unparsable = 1.0.
pub fn tempo(base: Duration) -> Duration {
    static FACTOR: std::sync::LazyLock<f32> = std::sync::LazyLock::new(|| {
        std::env::var("IRIS_SLOWMO")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0)
    });
    base.mul_f32(*FACTOR)
}

pub fn spring(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return 0.0;
    }
    // k=280, c=30: w=sqrt(280), zeta=c/(2w), wd=w*sqrt(1-zeta^2).
    // Precomputed: the sqrt/exp chain ran per call, per frame, per
    // animated element.
    const W: f32 = 16.733_2; // sqrt(280)
    const ZETA: f32 = 0.896_421; // 30 / (2*W)
    const WD: f32 = 7.416_198; // W * sqrt(1 - ZETA^2)
    let time = t * 0.55;
    // zeta<1 overshoots ~0.01%; GPUI's animation driver panics on
    // deltas outside 0..=1, so the curve clamps its output.
    (1.0 - (-ZETA * W * time).exp() * ((WD * time).cos() + (ZETA * W / WD) * (WD * time).sin()))
        .clamp(0.0, 1.0)
}

/// A value+velocity spring for pointer-coupled state. Unlike the
/// clock-based curve above, this retains velocity when a hover ends
/// mid-rise and reverses toward the new target. The exact critically
/// damped update depends on elapsed time, not frame count, and catches
/// up after a stalled frame without oscillating.
#[derive(Clone, Copy, Default)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
}

impl Spring {
    const OMEGA: f32 = 28.0;

    /// Advance toward `target` by `dt` seconds; returns the value.
    pub fn to(&mut self, target: f32, dt: f32) -> f32 {
        let dt = dt.max(0.0);
        let offset = self.value - target;
        let coefficient = self.velocity + Self::OMEGA * offset;
        let decay = (-Self::OMEGA * dt).exp();
        self.value = target + (offset + coefficient * dt) * decay;
        self.velocity = (self.velocity - Self::OMEGA * coefficient * dt) * decay;
        if self.settled(target) {
            self.value = target;
            self.velocity = 0.0;
        }
        self.value
    }

    pub fn settled(&self, target: f32) -> bool {
        (self.value - target).abs() < 0.002 && self.velocity.abs() < 0.02
    }
}

// WHY: the class closed here is "motion math breaks the animation
// contract": spring() must stay inside 0..=1 (GPUI's animation driver
// panics outside it) and Spring must converge without oscillation or
// refresh-rate-dependent timing.
// Not covered: compositor delivery and paint duration.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_curve_stays_in_unit_range() {
        for i in 0..=200 {
            let v = spring(i as f32 / 200.0);
            assert!(
                (0.0..=1.0).contains(&v),
                "spring({}) = {v}",
                i as f32 / 200.0
            );
        }
        assert_eq!(spring(0.0), 0.0);
        assert!((spring(1.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn integrator_converges_without_overshoot() {
        let mut s = Spring::default();
        let mut peak = 0.0f32;
        for _ in 0..600 {
            let v = s.to(1.0, 1.0 / 120.0);
            peak = peak.max(v);
        }
        assert!(s.settled(1.0), "value={} vel={}", s.value, s.velocity);
        assert!(peak <= 1.001, "critically damped spring overshot: {peak}");
    }

    #[test]
    fn integrator_reverses_mid_flight() {
        let mut s = Spring::default();
        for _ in 0..30 {
            s.to(1.0, 1.0 / 120.0);
        }
        let mid = s.value;
        assert!(mid > 0.1 && mid < 1.0);
        for _ in 0..600 {
            s.to(0.0, 1.0 / 120.0);
        }
        assert!(s.settled(0.0));
    }

    #[test]
    fn integrator_matches_elapsed_time_across_frame_rates() {
        for dt in [1.0_f32 / 120.0, 1.0 / 60.0, 1.0 / 30.0, 0.05, 0.1, 0.2] {
            let mut s = Spring::default();
            let steps = (0.2 / dt).round() as usize;
            for _ in 0..steps {
                s.to(1.0, dt);
            }
            let elapsed = steps as f32 * dt;
            let x = 28.0 * elapsed;
            let expected = 1.0 - (1.0 + x) * (-x).exp();
            assert!(
                (s.value - expected).abs() < 0.000_01,
                "dt={dt}: value={}, expected={expected}",
                s.value
            );
        }
    }

    #[test]
    fn integrator_settles_within_350ms_at_any_frame_rate() {
        for dt in [1.0 / 120.0, 1.0 / 60.0, 1.0 / 30.0, 0.05, 0.2] {
            let mut s = Spring::default();
            let mut elapsed = 0.0;
            while elapsed < 0.35 {
                s.to(1.0, dt);
                elapsed += dt;
            }
            assert!(
                s.settled(1.0),
                "dt={dt}: value={} velocity={}",
                s.value,
                s.velocity
            );
            assert_eq!(s.value, 1.0, "dt={dt}: rest must be exact");
            assert_eq!(s.velocity, 0.0, "dt={dt}: rest must have no velocity");
        }
    }

    #[test]
    fn stalled_frame_catches_up_to_the_current_time() {
        let mut s = Spring::default();
        s.to(1.0, 10.0);
        assert!(s.settled(1.0), "value={} velocity={}", s.value, s.velocity);
        assert_eq!(s.value, 1.0);
        assert_eq!(s.velocity, 0.0);
    }
}
