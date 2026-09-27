//! The recording chip's buttons, clicked on a private Xvfb.
//!
//! WHY: the class closed here is "a chip button acts on the recording
//! and the chip does not show it". A click runs inside the chip
//! window's event dispatch, where GPUI holds the window, and a chip
//! update or close through the chip's handle failed there: a pause
//! click paused the recording and left the dot red and the clock
//! running, and a stop click would have saved the recording and left
//! the chip open. The case records a window, clicks pause twice and
//! stop once with XTest, and reads the chip back: the pill's centre row
//! for the dot's colour, the window list and the recordings folder for
//! stop.
//!
//! Not covered: the mic button, which needs an audio source (its state
//! goes through the same update as the pause state); Wayland, which
//! opens no chip; Windows and macOS. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb and ffmpeg; without them
//! it prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The exit tests use the rest of the harness.
#[allow(dead_code)]
#[path = "support/recording.rs"]
mod recording;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, ImageFormat};

use recording::Recording;
use x11::{click, windows_of};

/// The chip window's title.
const CHIP: &str = "Recording - iris";
/// The chip window's transparent margin around the pill, and the pill's
/// size, in pixels at scale 1 (chip.rs).
const BLEED: i16 = 16;
const PILL: (i16, i16) = (208, 36);

/// The chip's centre row, read from its window.
struct Row {
    /// The chip window's root position.
    at: (i16, i16),
    /// Blue, green, red of each pill pixel.
    px: Vec<[u8; 3]>,
}

impl Row {
    fn read(conn: &impl Connection, root: u32, chip: u32) -> Option<Row> {
        let at = conn
            .translate_coordinates(chip, root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let image = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                chip,
                BLEED,
                BLEED + PILL.1 / 2,
                PILL.0 as u16,
                1,
                !0,
            )
            .ok()?
            .reply()
            .ok()?;
        // Xvfb stores depth 24 and 32 in 32 bits a pixel, blue first.
        assert_eq!(
            image.data.len(),
            PILL.0 as usize * 4,
            "depth {}",
            image.depth
        );
        let px = image.data.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
        Some(Row {
            at: (at.dst_x, at.dst_y),
            px,
        })
    }

    /// Whether a pixel on the row is red: the recording dot, which a
    /// paused chip draws grey.
    fn red(&self) -> bool {
        self.px
            .iter()
            .any(|&[b, g, r]| r > g.saturating_add(60) && r > b.saturating_add(60))
    }

    /// The runs of pixels that differ from the pill, as pill x ranges.
    /// Items on the row are 8 px apart; a gap of 5 px or less joins the
    /// parts of one glyph.
    fn runs(&self) -> Vec<(i16, i16)> {
        // Inside the pill's rounded ends, left of every item.
        let (start, end) = (PILL.1 / 2, PILL.0 - PILL.1 / 2);
        let bg = self.px[start as usize];
        let differs = |p: [u8; 3]| {
            p.iter()
                .zip(bg)
                .map(|(&a, b)| u32::from(a.abs_diff(b)))
                .sum::<u32>()
                > 60
        };
        let mut runs: Vec<(i16, i16)> = Vec::new();
        for x in start..end {
            if !differs(self.px[x as usize]) {
                continue;
            }
            match runs.last_mut() {
                Some(run) if x - run.1 <= 5 => run.1 = x,
                _ => runs.push((x, x)),
            }
        }
        runs
    }

    /// The root point at the middle of `run`.
    fn at(&self, run: (i16, i16)) -> (i16, i16) {
        (
            self.at.0 + BLEED + (run.0 + run.1) / 2,
            self.at.1 + BLEED + PILL.1 / 2,
        )
    }
}

#[test]
fn chip_buttons_pause_resume_and_stop_the_recording_they_show() {
    let case = "chip_buttons_pause_resume_and_stop_the_recording_they_show";
    let Some(rec) = Recording::start(case) else {
        return;
    };
    let (conn, root) = rec.x();
    let chip = rec.daemon.until("the chip to open", || {
        windows_of(conn, root, CHIP).first().copied()
    });
    let row = || Row::read(conn, root, chip);
    let row_that =
        |what: &str, f: &dyn Fn(&Row) -> bool| rec.daemon.until(what, || row().filter(|r| f(r)));

    // The dot, the timer (one run or more as its digits change), pause,
    // stop, and the mic button of an mp4 recording.
    let shown = row_that("the chip to draw its dot and three buttons", &|r| {
        r.red() && r.runs().len() >= 5
    });
    let runs = shown.runs();
    let (pause, stop) = (runs[runs.len() - 3], runs[runs.len() - 2]);

    click(conn, root, shown.at(pause));
    row_that("a pause click to turn the dot grey", &|r| !r.red());
    click(conn, root, shown.at(pause));
    row_that("a second pause click to turn it red again", &|r| r.red());

    click(conn, root, shown.at(stop));
    rec.daemon.until("a stop click to close the chip", || {
        windows_of(conn, root, CHIP).is_empty().then_some(())
    });
    rec.daemon.until("a stop click to save the recording", || {
        rec.daemon
            .log()
            .contains("iris: recording saved")
            .then_some(())
    });
    rec.saved();
}
