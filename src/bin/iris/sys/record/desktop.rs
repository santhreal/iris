//! Desktop recording on Windows and macOS: ffmpeg reads the main
//! display, or the display that holds a region, cropped to it. The chip
//! opens above the region.

use gpui::App;
use iris_lib::capture::WinRect;
use iris_lib::record::{self, ActiveRecording};

use super::Params;
use crate::chip;

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

fn start(cx: &mut App, p: Params, region: Option<WinRect>) -> Result<ActiveRecording, String> {
    let monitors = iris_lib::capture::monitors().unwrap_or_default();
    chip::open(cx, p.chip_mic(), region, &monitors)?;
    p.spawn(move |spec| record::desktop::record_desktop(spec, region))
}
