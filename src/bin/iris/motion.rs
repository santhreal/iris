//! Motion: the damped-spring curve every iris surface shares.
//!
//! Closed form, k=280, c=30, zeta ~0.9 (DESIGN.md). Normalized time
//! 0..1 maps to a ~0.55s settle; the output overshoots past 1 briefly
//! before settling, which is the point: callers drive it manually per
//! frame. GPUI's own animation driver panics on deltas outside 0..=1,
//! so nothing here goes through `with_animation`.

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
    let w: f32 = 280.0f32.sqrt();
    let zeta: f32 = 30.0 / (2.0 * w);
    let wd = w * (1.0 - zeta * zeta).sqrt();
    let time = t * 0.55;
    // zeta<1 overshoots ~0.01%; GPUI's animation driver panics on
    // deltas outside 0..=1, so the curve clamps its output.
    (1.0 - (-zeta * w * time).exp() * ((wd * time).cos() + (zeta * w / wd) * (wd * time).sin()))
        .clamp(0.0, 1.0)
}

/// A value+velocity spring for pointer-coupled state. Unlike the
/// clock-based curve above, this integrates per frame, so a hover
/// that ends mid-rise reverses from the current value WITH its
/// current velocity: the liquid feel. Critically damped (zeta=1):
/// it glides to rest with no overshoot, because an oscillating
/// shadow reads as jitter, not physics.
#[derive(Clone, Copy, Default)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
}

impl Spring {
    const K: f32 = 140.0;
    const C: f32 = 23.66; // 2*sqrt(K): critical damping

    /// Advance toward `target` by `dt` seconds; returns the value.
    pub fn to(&mut self, target: f32, dt: f32) -> f32 {
        let dt = dt.min(0.05); // a stalled frame must not explode it
        let force = -Self::K * (self.value - target) - Self::C * self.velocity;
        self.velocity += force * dt;
        self.value += self.velocity * dt;
        self.value
    }

    pub fn settled(&self, target: f32) -> bool {
        (self.value - target).abs() < 0.002 && self.velocity.abs() < 0.02
    }


}

// WHY: the class closed here is "motion math breaks the animation
// contract": spring() must stay inside 0..=1 (GPUI's animation driver
// panics outside it) and Spring must converge without oscillation.
// Not covered: wall-clock pacing, which is frame-scheduler territory.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_curve_stays_in_unit_range() {
        for i in 0..=200 {
            let v = spring(i as f32 / 200.0);
            assert!((0.0..=1.0).contains(&v), "spring({}) = {v}", i as f32 / 200.0);
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
    fn stalled_frame_dt_is_clamped() {
        let mut s = Spring::default();
        // A 10-second dt must not explode the integrator.
        let v = s.to(1.0, 10.0);
        assert!(v.is_finite() && v < 2.0);
    }
}
