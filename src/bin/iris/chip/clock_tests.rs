//! WHY: the class closed here is "the chip's clock counts time the file
//! does not have": a pause that does not stop the clock, a resume that
//! jumps it forward by the pause, a repeated pause that restarts the
//! pause, and pauses that do not add up. Not covered: the frame cadence
//! (a GPUI timer), and drift from the recorder's own timeline, which the
//! chip does not read.

use std::time::{Duration, Instant};

use super::Chip;

const S: Duration = Duration::from_secs(1);

#[test]
fn the_clock_runs_with_wall_time_while_recording() {
    let t0 = Instant::now();
    let chip = Chip::new(None, t0);
    assert_eq!(chip.elapsed(t0), Duration::ZERO);
    assert_eq!(chip.elapsed(t0 + 5 * S), 5 * S);
}

#[test]
fn a_pause_stops_the_clock_and_a_resume_continues_it() {
    let t0 = Instant::now();
    let mut chip = Chip::new(None, t0);
    chip.set_paused(true, t0 + 3 * S);
    assert_eq!(chip.elapsed(t0 + 3 * S), 3 * S);
    assert_eq!(chip.elapsed(t0 + 60 * S), 3 * S);
    chip.set_paused(false, t0 + 60 * S);
    assert_eq!(chip.elapsed(t0 + 60 * S), 3 * S);
    assert_eq!(chip.elapsed(t0 + 62 * S), 5 * S);
}

#[test]
fn pauses_add_up() {
    let t0 = Instant::now();
    let mut chip = Chip::new(None, t0);
    // Four rounds of 2s recorded, then 10s paused.
    for k in 0..4u32 {
        let round = t0 + k * 12 * S;
        chip.set_paused(true, round + 2 * S);
        chip.set_paused(false, round + 12 * S);
    }
    assert_eq!(chip.elapsed(t0 + 48 * S), 8 * S);
}

#[test]
fn a_repeated_pause_or_resume_changes_nothing() {
    let t0 = Instant::now();
    let mut chip = Chip::new(None, t0);
    chip.set_paused(false, t0 + S);
    assert_eq!(chip.elapsed(t0 + 4 * S), 4 * S);
    chip.set_paused(true, t0 + 4 * S);
    // The pause began at 4s, not at the second request.
    chip.set_paused(true, t0 + 9 * S);
    chip.set_paused(false, t0 + 10 * S);
    assert_eq!(chip.elapsed(t0 + 11 * S), 5 * S);
}
