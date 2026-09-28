//! The libraries' `log` records at warning level and above land in
//! iris.log, each on a line with its target and level; records below
//! warning do not.
//!
//! WHY: the class closed here is "a library's report of a failure leaves
//! no trace". iris installed no `log` logger, so the warnings and errors
//! of GPUI and wgpu (a lost GPU device, a surface configure that panicked,
//! a frame that failed to draw) went nowhere, and iris.log showed a window
//! that drew nothing with no cause. Not covered: a library that logs a
//! warning every frame; only the file's size cap bounds that.
//!
//! One case in its own test binary: it sets `IRIS_HOME` and installs the
//! process's logger, both process-wide.

use std::path::Path;

fn log_lines(home: &Path) -> Vec<String> {
    std::fs::read_to_string(home.join("state").join("iris.log"))
        .unwrap_or_default()
        .lines()
        // Past the "[hh:mm:ss.mmm] " stamp.
        .map(|line| {
            line.split_once("] ")
                .map_or(line, |(_, rest)| rest)
                .to_owned()
        })
        .collect()
}

#[test]
fn library_warnings_and_errors_reach_the_log_file_and_nothing_below() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("IRIS_HOME", home.path());
    iris_lib::log::init();
    // A second start keeps one logger, so each record is one line.
    iris_lib::log::init();

    log::error!(target: "wgpu_hal::vulkan", "device lost");
    log::warn!(target: "gpui_wgpu::wgpu_renderer", "surface configure failed");
    log::info!(target: "gpui_wgpu::wgpu_renderer", "context selected");
    log::debug!(target: "zbus", "message received");
    log::trace!(target: "calloop", "dispatch");
    // A record handed to the logger itself, past the macros' level check.
    log::logger().log(
        &log::Record::builder()
            .level(log::Level::Info)
            .target("wgpu_core")
            .args(format_args!("adapter selected"))
            .build(),
    );

    assert_eq!(
        log_lines(home.path()),
        [
            "wgpu_hal::vulkan: ERROR: device lost",
            "gpui_wgpu::wgpu_renderer: WARN: surface configure failed",
        ]
    );
}
