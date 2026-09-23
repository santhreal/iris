//! Desktop recording on Windows and macOS: ffmpeg reads the main
//! display, or the display that holds a region, cropped to it. The chip
//! opens above the region; the source closes it when it ends, so an
//! ffmpeg failure does not leave it on screen.

use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::config::RecordingFormat;
use iris_lib::record::{self, ActiveRecording};

use super::Params;
use crate::chip;
use crate::daemon::{command_tx, Command};

pub(super) fn region_available() -> Result<(), String> {
    Ok(())
}

pub(super) fn start_window(cx: &mut App, p: Params) -> Result<ActiveRecording, String> {
    start(cx, p, None)
}

pub(super) fn start_region(
    cx: &mut App,
    p: Params,
    rect: WinRect,
) -> Result<ActiveRecording, String> {
    start(cx, p, Some(rect))
}

fn start(cx: &mut App, mut p: Params, region: Option<WinRect>) -> Result<ActiveRecording, String> {
    // GIF and WebM carry no audio track; the chip shows what is recorded.
    p.mic = p.mic && p.format == RecordingFormat::Mp4;
    chip::open(cx, p.mic, region)?;
    let done = command_tx();
    Ok(p.spawn(move |spec| {
        let result = record::desktop::record_desktop(spec, region);
        if let Some(tx) = &done {
            let _ = tx.unbounded_send(Command::ChipHide);
        }
        result
    }))
}
