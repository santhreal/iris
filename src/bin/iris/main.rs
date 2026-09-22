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
//!   --record-pause       pause/resume the active recording
//!   --record-mic         toggle the mic on the active recording
//!   --library            open the library panel
//!   --settings           open the settings window
//!   --home               open the home surface
//!   --annotate <file>    edit an existing capture
//!   --toast <file.png>   debug: show the toast stage for an existing file
//!   --quit               ask the daemon to exit
//!   --version            print the version and exit
//!   --check-update       report whether a newer release exists
//!   --update             download and apply the latest release

mod chip;
mod daemon;
mod editor;
mod flash;
mod home;
mod icons;
mod library;
mod motion;
mod overlay;
mod pin;
mod pipeline;
mod settings;
mod stage;
mod sys;
mod theme;
mod update;
mod widgets;

use gpui::*;

fn main() {
    iris_lib::log::init();
    // Skip argv[0]; daemon::parse_args handles the flags.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Informational and self-update flags run in this process, not the
    // daemon: they print to the caller's stdout and `--update` replaces
    // the binary, so forwarding them to a running daemon would both
    // hide the output and let the daemon overwrite its own exe.
    if args.iter().any(|a| a == "--version") {
        println!("iris {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--check-update") {
        match update::check() {
            Ok(Some(info)) => println!("iris: update available: {}", info.version),
            Ok(None) => println!("iris: up to date ({})", env!("CARGO_PKG_VERSION")),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if args.iter().any(|a| a == "--update") {
        let result = match update::check() {
            Ok(Some(info)) => update::apply(&info),
            Ok(None) => {
                println!("iris: up to date ({})", env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    if sys::ipc::forward_if_running(&args) {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        theme::load_fonts(cx);
        daemon::start(cx);
        for cmd in daemon::parse_args(&args) {
            if let Err(e) = daemon::dispatch(cx, &cmd) {
                iris_lib::ilog!("iris: {e}");
            }
        }
    });
}
