//! iris on GPUI: native UI shell.
//!
//! Layout: this
//! binary shares the backend modules with the Tauri build through
//! iris_lib and reimplements every surface natively. The first
//! process becomes the daemon (tray, hotkeys, single-instance socket);
//! later invocations forward their flags to it and exit.
//!
//! CLI:
//!   --capture            region capture: frozen-frame overlay, then toast
//!   --capture-fullscreen full-screen grab with no overlay, then toast
//!   --capture-window     capture the focused window, then toast
//!   --delay <secs>       full-screen capture after a countdown
//!   --record-window      toggle a window-picked recording
//!   --record-region      pick a screen region and record it
//!   --library            open the library panel
//!   --settings           open the settings window
//!   --home               open the home surface
//!   --annotate <file>    edit an existing capture
//!   --toast <file.png>   debug: show the toast stage for an existing file
//!   --quit               ask the daemon to exit

mod chip;
mod daemon;
mod home;
mod flash;
mod icons;
mod motion;
mod editor;
mod overlay;
mod library;
mod pin;
mod pipeline;
mod settings;
mod stage;
mod theme;
mod widgets;
mod xwin;

use gpui::*;

fn main() {
    iris_lib::log::init();
    // Skip argv[0]; daemon::parse_args handles the flags.
    let args: Vec<String> = std::env::args().skip(1).collect();

    if daemon::forward_if_running(&args) {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        // Inter, bundled: the register is SF Pro, and fontconfig's
        // default sans on Linux (DejaVu) cannot carry it. Loaded
        // into the text system so the family resolves on machines
        // without Inter installed; surfaces set font_family("Inter")
        // on their root and the style cascades.
        if let Err(e) = cx.text_system().add_fonts(vec![
            std::borrow::Cow::Borrowed(
                include_bytes!("../../../assets/fonts/Inter-Regular.otf") as &[u8],
            ),
            std::borrow::Cow::Borrowed(
                include_bytes!("../../../assets/fonts/Inter-Medium.otf") as &[u8],
            ),
            std::borrow::Cow::Borrowed(
                include_bytes!("../../../assets/fonts/Inter-SemiBold.otf") as &[u8],
            ),
            std::borrow::Cow::Borrowed(
                include_bytes!("../../../assets/fonts/Inter-Bold.otf") as &[u8],
            ),
        ]) {
            iris_lib::ilog!("iris: fonts: {e}");
        }
        daemon::start(cx);
        for cmd in daemon::parse_args(&args) {
            if let Err(e) = daemon::dispatch(cx, &cmd) {
                iris_lib::ilog!("iris: {e}");
            }
        }
    });
}
