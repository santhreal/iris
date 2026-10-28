//! Global hotkeys on Wayland through the GlobalShortcuts portal.
//!
//! A Wayland compositor grabs no key for a client, so the chords are
//! bound through `org.freedesktop.portal.GlobalShortcuts`: the session
//! asks for each configured chord as its preferred trigger, the desktop
//! confirms or reassigns them (a dialog on first use), and a press
//! arrives as an `Activated` signal. One thread holds the portal for
//! the daemon's life. A settings save closes the session and binds a
//! new one with the new chords.

use std::pin::pin;

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use ashpd::desktop::Session;
use futures::channel::mpsc::{unbounded, UnboundedReceiver, UnboundedSender};
use futures::future::{select, Either};
use futures::StreamExt;
use iris_lib::config::Config;
use parking_lot::Mutex;

use super::super::{configured, record, refuse_all, Binding, Chord, UNKNOWN_KEY};
use crate::daemon::Command;

/// No GlobalShortcuts portal on the session bus: GNOME before 48, and
/// sessions without xdg-desktop-portal.
const NO_PORTAL: &str = "This desktop has no global shortcut service";
/// The desktop's dialog declined the chord, or the portal failed it.
const DECLINED: &str = "Not approved by the desktop";

/// Wakes the portal thread to bind the chords again.
static REGRAB: Mutex<Option<UnboundedSender<()>>> = Mutex::new(None);

/// Start the portal thread. The binding is asynchronous: the chords go
/// live once the desktop answers, which may wait on its dialog.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    let (regrab_tx, regrab_rx) = unbounded();
    *REGRAB.lock() = Some(regrab_tx);
    let spawned = std::thread::Builder::new()
        .name("iris-shortcuts".into())
        .spawn(move || futures::executor::block_on(run(tx, regrab_rx)));
    if let Err(e) = spawned {
        iris_lib::ilog!("iris: hotkeys: spawn portal thread: {e}");
        refuse_all(NO_PORTAL);
    }
}

pub(super) fn request_regrab() {
    if let Some(tx) = REGRAB.lock().as_ref() {
        let _ = tx.unbounded_send(());
    }
}

async fn run(tx: UnboundedSender<Command>, mut regrab: UnboundedReceiver<()>) {
    let portal = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(e) => {
            iris_lib::ilog!("iris: hotkeys: GlobalShortcuts portal unavailable: {e}");
            refuse_all(NO_PORTAL);
            // A settings save still records the new chords' state.
            while regrab.next().await.is_some() {
                refuse_all(NO_PORTAL);
            }
            return;
        }
    };
    let activated = match portal.receive_activated().await {
        Ok(s) => s,
        Err(e) => {
            iris_lib::ilog!("iris: hotkeys: GlobalShortcuts signal: {e}");
            refuse_all(NO_PORTAL);
            return;
        }
    };
    let mut activated = pin!(activated);
    loop {
        let session = bind(&portal).await;
        let rebind = loop {
            match select(activated.next(), regrab.next()).await {
                Either::Left((Some(ev), _)) => {
                    // The previous session closed before this one
                    // bound, so every press is this session's.
                    let chord = Chord::ALL
                        .into_iter()
                        .find(|c| c.portal().0 == ev.shortcut_id());
                    if let Some(chord) = chord {
                        if tx.unbounded_send(chord.command()).is_err() {
                            break false;
                        }
                    }
                }
                Either::Right((Some(()), _)) => break true,
                Either::Left((None, _)) | Either::Right((None, _)) => break false,
            }
        };
        // Saves that queued during the bind collapse into one rebind.
        while regrab.try_recv().is_ok() {}
        if let Some(s) = session {
            let _ = s.close().await;
        }
        if !rebind {
            return;
        }
    }
}

/// Bind the configured chords in a new session and record each one's
/// outcome. `None` when no chord needs the portal or the session failed.
async fn bind(portal: &GlobalShortcuts) -> Option<Session<GlobalShortcuts>> {
    let cfg = Config::load();
    let mut wanted = Vec::new();
    for (chord, keys) in configured(&cfg) {
        if keys.is_empty() {
            record(chord, keys, Binding::Unset);
            continue;
        }
        let Some(trigger) = trigger(keys) else {
            record(chord, keys, Binding::Refused(UNKNOWN_KEY.into()));
            continue;
        };
        let (id, description) = chord.portal();
        let shortcut = NewShortcut::new(id, description).preferred_trigger(trigger.as_str());
        wanted.push((chord, keys, shortcut));
    }
    if wanted.is_empty() {
        return None;
    }
    let refuse = |why: &str| {
        for (chord, keys, _) in &wanted {
            record(*chord, keys, Binding::Refused(why.to_owned()));
        }
    };
    let session = match portal.create_session(Default::default()).await {
        Ok(s) => s,
        Err(e) => {
            iris_lib::ilog!("iris: hotkeys: GlobalShortcuts session: {e}");
            refuse(NO_PORTAL);
            return None;
        }
    };
    let shortcuts: Vec<NewShortcut> = wanted.iter().map(|w| w.2.clone()).collect();
    let bound = portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await
        .and_then(|request| request.response());
    match bound {
        Ok(bound) => {
            for (chord, keys, _) in &wanted {
                let id = chord.portal().0;
                let live = bound.shortcuts().iter().any(|s| s.id() == id);
                let binding = if live {
                    Binding::Live
                } else {
                    Binding::Refused(DECLINED.into())
                };
                record(*chord, keys, binding);
            }
        }
        Err(e) => {
            iris_lib::ilog!("iris: hotkeys: GlobalShortcuts bind: {e}");
            refuse(DECLINED);
        }
    }
    Some(session)
}

/// A chord in the hotkey grammar ("Ctrl+Shift+R") as a trigger in the
/// XDG shortcuts grammar ("CTRL+SHIFT+r"): modifier names upper case,
/// the key as its xkb keysym name. `None` for a chord with no key or
/// a key outside the grammar.
fn trigger(keys: &str) -> Option<String> {
    let mut mods: Vec<&str> = Vec::new();
    let mut key = None;
    for tok in keys.split('+').map(str::trim).filter(|t| !t.is_empty()) {
        let lower = tok.to_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some("CTRL"),
            "shift" => Some("SHIFT"),
            "alt" | "option" => Some("ALT"),
            "super" | "cmd" | "command" | "meta" | "win" | "windows" => Some("LOGO"),
            _ => None,
        };
        match modifier {
            Some(m) if !mods.contains(&m) => mods.push(m),
            Some(_) => {}
            None => key = Some(keysym_name(&lower)?),
        }
    }
    let key = key?;
    let mut out = String::new();
    for m in ["CTRL", "ALT", "SHIFT", "LOGO"] {
        if mods.contains(&m) {
            out.push_str(m);
            out.push('+');
        }
    }
    out.push_str(&key);
    Some(out)
}

/// The xkb keysym name of a lower-cased key in the hotkey grammar.
fn keysym_name(key: &str) -> Option<String> {
    let named = match key {
        "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" => "Print",
        "escape" | "esc" => "Escape",
        "enter" | "return" => "Return",
        "space" | "spacebar" => "space",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "delete" | "del" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "Page_Up",
        "pagedown" | "pgdn" => "Page_Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => return Some(k.into()),
        k => {
            let n: u32 = k.strip_prefix('f')?.parse().ok()?;
            return (1..=24).contains(&n).then(|| format!("F{n}"));
        }
    };
    Some(named.into())
}

// WHY: the class closed here is "a Wayland chord asks the desktop for
// a trigger it cannot parse". The XDG shortcuts grammar names keys by
// xkb keysym and modifiers in upper case; a trigger built from the
// config text as typed ("Ctrl+Shift+R") is not a preferred trigger the
// desktop can honor. Every key of the hotkey grammar maps to its keysym
// name, modifiers are canonical and deduplicated, and a key outside the
// grammar binds nothing instead of a wrong chord. Not covered: the
// portal round trip, which needs a desktop with the portal.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn chords_become_xdg_triggers() {
        for (keys, want) in [
            ("Ctrl+Shift+R", Some("CTRL+SHIFT+r")),
            ("shift+ctrl+r", Some("CTRL+SHIFT+r")),
            ("Print", Some("Print")),
            ("Super+PrtSc", Some("LOGO+Print")),
            ("Alt+F12", Some("ALT+F12")),
            ("Cmd+Option+5", Some("ALT+LOGO+5")),
            ("Ctrl+Ctrl+PageDown", Some("CTRL+Page_Down")),
            (" Ctrl + Space ", Some("CTRL+space")),
            ("Ctrl+Shift", None),
            ("Ctrl+F25", None),
            ("Ctrl+F0", None),
            ("Ctrl+@", None),
            ("Ctrl+Hyper", None),
            ("", None),
        ] {
            assert_eq!(trigger(keys).as_deref(), want, "{keys:?}");
        }
    }
}
