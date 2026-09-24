//! iris on GPUI: native UI shell.
//!
//! This binary drives the backend modules in `iris_lib` and renders
//! every surface natively. One process is the daemon (tray, hotkeys,
//! single-instance socket); other invocations forward their flags to it
//! and exit. A flagged invocation with no daemon running starts a
//! detached `iris --daemon` first, except `--quit`, `--record-pause` and
//! `--record-mic`, which only reach a daemon that is already up.
//!
//! `cli::OPTS` defines every option; `iris --help` prints them.

// Windows starts a GUI program with no console window: none opens for
// the daemon at login or for a Start Menu or hotkey launch. A command
// line in a terminal prints through sys::console. Other targets ignore
// the attribute.
#![windows_subsystem = "windows"]

mod chip;
mod cli;
mod daemon;
mod editor;
mod flash;
mod home;
mod icons;
mod library;
mod motion;
mod notice;
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
    sys::alloc::tune();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = cli::parse(&args);
    // A bare `iris` and `iris --daemon` print nothing; any other command
    // line prints its output and errors.
    if parsed.prints() {
        sys::console::attach();
    }
    iris_lib::log::init();

    let local = parsed.first_local();
    // A mistyped command line runs nothing: no daemon starts and a
    // running one receives nothing. `--help` still prints.
    if !parsed.errors.is_empty() && local != Some(cli::Local::Help) {
        for e in &parsed.errors {
            eprintln!("iris: {e}");
        }
        eprintln!("iris: run 'iris --help' for the options");
        std::process::exit(2);
    }
    if let Some(local) = local {
        run_local(local);
        return;
    }

    let claimed = if parsed.daemon {
        sys::ipc::claim()
    } else {
        // Quit, pause, and the mic toggle act on a running daemon. With
        // none up there is nothing to act on, and spawning one to deliver
        // them would start a daemon only to stop it: an installer's
        // pre-upgrade `iris --quit` would then hold the binary open while
        // it copies. A quit with nothing running already holds; pause and
        // mic fail.
        if daemon::live_daemon_only(&parsed.cmds) {
            if !sys::ipc::send_to_daemon(&parsed.forward)
                && !parsed
                    .cmds
                    .iter()
                    .all(|c| matches!(c, daemon::Command::Quit))
            {
                eprintln!("iris: no iris daemon is running");
                std::process::exit(1);
            }
            return;
        }
        sys::ipc::forward_if_running(&parsed.forward)
    };
    let claimed = match claimed {
        Ok(Some(claimed)) => claimed,
        // Delivered, or `--daemon` found a daemon that runs or starts.
        Ok(None) => {
            if parsed.daemon {
                iris_lib::ilog!("iris: another iris daemon runs or is starting; this one exits");
            }
            return;
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let cmds = parsed.cmds;

    // No thread starts before this: on a Wayland session gpui creates the
    // Vulkan instance with DISPLAY hidden from the drivers only while the
    // process has one thread, and the NVIDIA driver connects to DISPLAY
    // when it is loaded, which starts an on-demand XWayland.
    let app = Application::new();
    // Opening iris.app while the daemon runs starts no second process:
    // macOS reports a reopen to the daemon, which then shows home. The
    // other platforms never report one.
    app.on_reopen(|cx| daemon::run(cx, &daemon::Command::Home));
    app.run(move |cx: &mut App| {
        theme::load_fonts(cx);
        daemon::start(cx, claimed);
        for cmd in &cmds {
            daemon::run(cx, cmd);
        }
    });
}

/// Run an option the client handles in this process. They print to the
/// caller's terminal, and `--update` replaces the binary, so none is
/// forwarded: a running daemon would hide the output and could
/// overwrite its own executable.
fn run_local(local: cli::Local) {
    let version = env!("CARGO_PKG_VERSION");
    let result = match local {
        cli::Local::Help => {
            say(format_args!("{}", cli::usage()));
            Ok(())
        }
        cli::Local::Version => {
            say(format_args!("iris {version}\n"));
            Ok(())
        }
        cli::Local::CheckUpdate => update::check().map(|found| match found {
            Some(info) => say(format_args!("iris: update available: {}\n", info.version)),
            None => say(format_args!("iris: up to date ({version})\n")),
        }),
        cli::Local::Update => match update::check() {
            Ok(Some(info)) => update::apply(&info),
            Ok(None) => {
                say(format_args!("iris: up to date ({version})\n"));
                Ok(())
            }
            Err(e) => Err(e),
        },
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

/// Print to standard output. When stdout fails, as when its reader
/// closed the pipe before the command wrote, the command exits with
/// status 1 and prints no panic.
fn say(text: std::fmt::Arguments) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    if out.write_fmt(text).and_then(|()| out.flush()).is_err() {
        std::process::exit(1);
    }
}
