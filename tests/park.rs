//! WHY: the daemon opens the capture overlay as it starts and parks it,
//! and parks it again after each capture, so the next capture reuses the
//! live window. A park minimized the window, and a minimize is a request
//! to the window manager (ICCCM WM_CHANGE_STATE): on an X11 screen with
//! no window manager the overlay stayed mapped over every monitor, black
//! where no compositing manager ran. The class closed here is a parked
//! overlay that stays on a screen with no window manager, after the
//! warmup or after a capture, and a parked overlay that a capture then
//! fails to bring back. It also closes "the warm overlay takes input
//! it was never shown for": the warmup maps the overlay over the whole
//! screen before the park, and while mapped it took a click meant for
//! Home, and a record pick's click resolved to it and recorded the
//! invisible overlay. The warm overlay's input shape must be empty, and
//! a capture must restore it.
//!
//! The first case runs on a private Xvfb with no window manager. The
//! others put a window manager on it, reduced to what a park meets. A
//! park minimized the overlay there, and a minimized window stays
//! managed: the taskbar, the window switcher, and the GNOME overview
//! listed it, and choosing it there restored it. iris also raised the
//! warmup's overlay again once the window manager listed it, and 200 ms
//! later, to span the screen, and xfwm4 restores a minimized window that
//! is raised while another iris window has the focus, so the library
//! opened during the warmup showed the dimmed capture overlay over the
//! desktop. A park now withdraws the overlay (ICCCM 4.1.4). This window
//! manager answers a map request `MAP_DELAY` late, as a busy one does, so
//! the warmup's park reaches it before it manages the window, where only
//! the withdrawal's synthetic UnmapNotify tells it to drop the request.
//! xfwm4 starting as the daemon starts misses that withdrawal and keeps
//! the overlay mapped over the desktop; the third case's window manager
//! misses it too, and iris withdraws the overlay again once the window
//! manager lists it. The class closed there is a parked overlay the
//! window manager still manages or that iris restacks, after the warmup
//! or after a capture, a capture's overlay withdrawn while in use, and
//! an overlay mapped for a capture without listing, and then asking for,
//! the states that keep it above and out of taskbars and window
//! switchers. Not covered: a real window manager's response to these
//! requests; a park that races a fixup the window manager has not yet
//! listed the window for, which the fixup queue's unit test covers;
//! Wayland, Windows, and macOS, which destroy the overlay in place of a
//! park. The cases run only with `IRIS_X11_TEST_DISPLAY` set and need
//! `Xvfb`; without it a case prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as _, Redirect};
use x11rb::protocol::damage::{ConnectionExt as _, ReportLevel};
use x11rb::protocol::shape::{ConnectionExt as _, SK};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ConfigWindow, ConfigureWindowAux, ConnectionExt as _,
    EventMask, ImageFormat, InputFocus, PropMode,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

use daemon::{enabled, xvfb, Daemon};
use x11::{all_windows_of, focus, tap, windows_of, Compositor};

/// The title of the capture overlay's window.
const OVERLAY: &str = "Capture - iris";

/// The keysym of `Escape`, which cancels a capture.
const ESCAPE: u32 = 0xff1b;

/// How long the stand-in window manager (`window_manager`) takes to
/// answer a map request.
const MAP_DELAY: Duration = Duration::from_millis(100);

/// The rectangles of `window`'s input shape: none passes every click
/// through, one is the default, the window's whole rect.
fn input_shape(conn: &impl Connection, window: u32) -> usize {
    conn.shape_get_rectangles(window, SK::INPUT)
        .unwrap()
        .reply()
        .unwrap()
        .rectangles
        .len()
}

/// What the stand-in window manager (`window_manager`) saw.
#[derive(Default)]
struct Seen {
    /// The windows it manages, in `_NET_CLIENT_LIST` order.
    listed: Vec<u32>,
    /// Each window it mapped on request, with the states its
    /// `_NET_WM_STATE` property held then.
    mapped: Vec<(u32, Vec<u32>)>,
    /// Each window withdrawn from it (ICCCM 4.1.4), in order.
    withdrawn: Vec<u32>,
    /// Each window that asked to be minimized (ICCCM WM_CHANGE_STATE).
    minimized: Vec<u32>,
    /// Each state a `_NET_WM_STATE` add request asked for, in order: its
    /// window and the state's atom.
    added: Vec<(u32, u32)>,
    /// Each withdrawal it missed: the window it went on to manage.
    missed: Vec<u32>,
    /// Each restack request for a withdrawn window: its window and the
    /// request.
    restacks: Vec<(u32, &'static str)>,
}

/// A window manager on `display`, on a thread of its own until `stop`
/// is set. It maps a window `MAP_DELAY` after the window asks, focuses
/// it, and lists it in the root's `_NET_CLIENT_LIST`; on an UnmapNotify
/// for a listed window or one waiting to be mapped, the real one or the
/// synthetic one ICCCM 4.1.4 asks a withdrawing client to send, it
/// unmaps the window and drops it from the list. With `misses` set it
/// misses the first withdrawal of a window waiting to be mapped and maps
/// the window anyway, as a window manager does that is starting as the
/// window is withdrawn. It focuses a listed window that asks to be
/// active (`_NET_ACTIVE_WINDOW`). It records each map, withdrawal,
/// missed withdrawal, minimize request, and state a `_NET_WM_STATE` add
/// request asks for, and each restack of a withdrawn window: a
/// ConfigureWindow with a stack mode, or a `_NET_WM_STATE` request for
/// `_NET_WM_STATE_ABOVE`. It holds SubstructureRedirect on the root
/// before it returns.
fn window_manager(display: &str, stop: Arc<AtomicBool>, misses: bool) -> Arc<Mutex<Seen>> {
    let (wm, screen) = x11rb::connect(Some(display)).unwrap();
    let root = wm.setup().roots[screen].root;
    let redirect = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    wm.change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(redirect))
        .unwrap()
        .check()
        .unwrap();
    let atom = |name: &[u8]| wm.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let (client_list, change_state, net_state, above, active) = (
        atom(b"_NET_CLIENT_LIST"),
        atom(b"WM_CHANGE_STATE"),
        atom(b"_NET_WM_STATE"),
        atom(b"_NET_WM_STATE_ABOVE"),
        atom(b"_NET_ACTIVE_WINDOW"),
    );
    let seen = Arc::new(Mutex::new(Seen::default()));
    let log = seen.clone();
    std::thread::spawn(move || {
        // Map requests not yet answered: when each is due, its window.
        let mut waiting: Vec<(Instant, u32)> = Vec::new();
        let list = |listed: &[u32]| {
            let _ = wm.change_property32(
                PropMode::REPLACE,
                root,
                client_list,
                AtomEnum::WINDOW,
                listed,
            );
        };
        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            let mut seen = log.lock();
            for (_, window) in waiting.extract_if(.., |w| w.0 <= now) {
                let states = wm
                    .get_property(false, window, net_state, AtomEnum::ATOM, 0, 32)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok())
                    .and_then(|reply| reply.value32().map(Iterator::collect))
                    .unwrap_or_default();
                seen.mapped.push((window, states));
                let _ = wm.map_window(window);
                let _ = wm.set_input_focus(InputFocus::PARENT, window, CURRENT_TIME);
                seen.listed.push(window);
                list(&seen.listed);
            }
            let Ok(event) = wm.poll_for_event() else {
                return;
            };
            let Some(event) = event else {
                drop(seen);
                let _ = wm.flush();
                std::thread::sleep(Duration::from_millis(2));
                continue;
            };
            let withdrawn =
                |seen: &Seen, w: u32| seen.withdrawn.contains(&w) && !seen.listed.contains(&w);
            match event {
                Event::MapRequest(ev) => {
                    if !seen.listed.contains(&ev.window) && waiting.iter().all(|w| w.1 != ev.window)
                    {
                        waiting.push((now + MAP_DELAY, ev.window));
                    }
                }
                Event::UnmapNotify(ev)
                    if ev.event == root
                        && (seen.listed.contains(&ev.window)
                            || waiting.iter().any(|w| w.1 == ev.window)) =>
                {
                    if misses && seen.missed.is_empty() && !seen.listed.contains(&ev.window) {
                        seen.missed.push(ev.window);
                    } else {
                        waiting.retain(|w| w.1 != ev.window);
                        let _ = wm.unmap_window(ev.window);
                        seen.listed.retain(|&w| w != ev.window);
                        seen.withdrawn.push(ev.window);
                        list(&seen.listed);
                    }
                }
                Event::ConfigureRequest(ev) => {
                    if ev.value_mask.contains(ConfigWindow::STACK_MODE)
                        && withdrawn(&seen, ev.window)
                    {
                        seen.restacks
                            .push((ev.window, "ConfigureWindow stack mode"));
                    }
                    let aux = ConfigureWindowAux::from_configure_request(&ev);
                    let _ = wm.configure_window(ev.window, &aux);
                }
                Event::ClientMessage(ev) => {
                    let data = ev.data.as_data32();
                    if ev.type_ == change_state && data[0] == 3 {
                        seen.minimized.push(ev.window);
                    } else if ev.type_ == active && seen.listed.contains(&ev.window) {
                        let _ = wm.set_input_focus(InputFocus::PARENT, ev.window, CURRENT_TIME);
                    } else if ev.type_ == net_state {
                        // Action 1 is _NET_WM_STATE_ADD; a second state of
                        // 0 is none.
                        if data[0] == 1 {
                            let states = data[1..3].iter().filter(|&&s| s != 0);
                            let added = states.map(|&s| (ev.window, s)).collect::<Vec<_>>();
                            seen.added.extend(added);
                        }
                        if data[1..3].contains(&above) && withdrawn(&seen, ev.window) {
                            seen.restacks.push((ev.window, "_NET_WM_STATE_ABOVE"));
                        }
                    }
                }
                _ => {}
            }
            let _ = wm.flush();
        }
    });
    seen
}

#[test]
fn a_parked_overlay_leaves_a_screen_with_no_window_manager() {
    if !enabled() {
        return;
    }
    let case = "a_parked_overlay_leaves_a_screen_with_no_window_manager";
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let warm = daemon.until("the warmup to open the overlay", || {
        all_windows_of(&conn, root, OVERLAY).first().copied()
    });
    // A daemon starved of CPU reads as idle, so the park is awaited, and
    // the overlay checked again once the daemon is idle after it.
    daemon.settle();
    daemon.until("the warmup to park the overlay", || {
        windows_of(&conn, root, OVERLAY).is_empty().then_some(())
    });
    daemon.settle();
    let shown = windows_of(&conn, root, OVERLAY);
    assert!(
        shown.is_empty(),
        "the warmup left the overlay mapped: {shown:x?}"
    );
    assert_eq!(
        input_shape(&conn, warm),
        0,
        "the warm overlay takes pointer input while it is not in use"
    );

    for capture in 1..=2 {
        daemon.send(&["--capture"]);
        let focused = daemon.until(
            &format!("capture {capture}'s overlay to take the focus"),
            || {
                let focused = focus(&conn);
                windows_of(&conn, root, OVERLAY)
                    .contains(&focused)
                    .then_some(focused)
            },
        );
        assert_eq!(
            focused, warm,
            "capture {capture} opened another overlay window"
        );
        assert_ne!(
            input_shape(&conn, warm),
            0,
            "capture {capture}'s overlay lets pointer input through"
        );
        tap(&conn, root, ESCAPE);
        daemon.until(&format!("capture {capture}'s overlay to park"), || {
            windows_of(&conn, root, OVERLAY).is_empty().then_some(())
        });
    }
}

#[test]
fn a_parked_overlay_is_withdrawn_and_comes_back_unlisted_under_a_window_manager() {
    withdrawn_and_back(
        "a_parked_overlay_is_withdrawn_and_comes_back_unlisted_under_a_window_manager",
        false,
    );
}

#[test]
fn a_parked_overlay_a_starting_window_manager_manages_is_withdrawn_again() {
    withdrawn_and_back(
        "a_parked_overlay_a_starting_window_manager_manages_is_withdrawn_again",
        true,
    );
}

/// Park the overlay at the warmup and after two captures under the
/// stand-in window manager, which misses the warmup's withdrawal when
/// `misses` is set, and check each park and each capture.
fn withdrawn_and_back(case: &str, misses: bool) {
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    let stop = Arc::new(AtomicBool::new(false));
    let seen = window_manager(&display, stop.clone(), misses);
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let warm = daemon.until("the warmup to open the overlay", || {
        all_windows_of(&conn, root, OVERLAY).first().copied()
    });
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let states = [
        atom(b"_NET_WM_STATE_ABOVE"),
        atom(b"_NET_WM_STATE_SKIP_TASKBAR"),
        atom(b"_NET_WM_STATE_SKIP_PAGER"),
    ];
    // After the warmup's park and each capture's: withdrawn, never
    // minimized, unmapped, and sent no restack, past the second span
    // assertion 200 ms after the window manager lists the window and the
    // 50 ms rescan bound. A capture's restore spans the withdrawn window
    // before it maps it, so restacks count from each withdrawal on.
    let parked = |when: &str, parks: usize| {
        let since = daemon.until(&format!("{when} to withdraw the overlay"), || {
            let seen = seen.lock();
            let withdrawals = seen.withdrawn.iter().filter(|&&w| w == warm).count();
            (withdrawals == parks && !seen.listed.contains(&warm)).then_some(seen.restacks.len())
        });
        std::thread::sleep(Duration::from_millis(1000));
        let seen = seen.lock();
        assert_eq!(
            &seen.restacks[since..],
            &[] as &[(u32, &str)],
            "iris restacked the overlay {warm:#x} after {when} parked it"
        );
        assert_eq!(
            seen.minimized,
            Vec::<u32>::new(),
            "{when} minimized the overlay, which keeps it managed and listed"
        );
        assert!(
            !seen.listed.contains(&warm),
            "the window manager manages the overlay after {when} parked it"
        );
        assert!(
            windows_of(&conn, root, OVERLAY).is_empty(),
            "the overlay is mapped after {when} parked it"
        );
    };
    parked("the warmup", 1);
    assert_eq!(
        seen.lock().missed,
        if misses { vec![warm] } else { vec![] },
        "the window manager missed another withdrawal than the warmup's"
    );

    for capture in 1..=2 {
        let asked = seen.lock().added.len();
        daemon.send(&["--capture"]);
        let focused = daemon.until(
            &format!("capture {capture}'s overlay to take the focus"),
            || {
                let focused = focus(&conn);
                windows_of(&conn, root, OVERLAY)
                    .contains(&focused)
                    .then_some(focused)
            },
        );
        assert_eq!(
            focused, warm,
            "capture {capture} opened another overlay window"
        );
        let mapped = seen
            .lock()
            .mapped
            .iter()
            .rev()
            .find(|m| m.0 == warm)
            .map(|m| m.1.clone());
        let missing: Vec<u32> = states
            .iter()
            .copied()
            .filter(|s| !mapped.as_ref().is_some_and(|m| m.contains(s)))
            .collect();
        assert_eq!(
            missing,
            Vec::<u32>::new(),
            "capture {capture}'s overlay was mapped without these states \
             (above, skip taskbar, skip pager): {mapped:?}"
        );
        // A window manager may replace the states a window lists before
        // its map with its own, so the overlay asks for them again once
        // the window manager manages it.
        daemon.until(
            &format!("capture {capture}'s overlay to ask for its states"),
            || {
                let seen = seen.lock();
                let added: Vec<u32> = seen.added[asked..]
                    .iter()
                    .filter(|a| a.0 == warm)
                    .map(|a| a.1)
                    .collect();
                states.iter().all(|s| added.contains(s)).then_some(())
            },
        );
        // Past the window manager listing the overlay, and the
        // dispatcher's read of the list it changed.
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            seen.lock().listed.contains(&warm) && windows_of(&conn, root, OVERLAY) == [warm],
            "capture {capture}'s overlay was withdrawn while in use"
        );
        tap(&conn, root, ESCAPE);
        parked(&format!("capture {capture}"), capture + 1);
    }
    stop.store(true, Ordering::Relaxed);
}

// WHY: a compositor animates a withdrawn window from the last frame it
// drew. GNOME Shell scales a closing window down and fades it out over
// 150 ms, so a park that withdrew the overlay as the capture ended
// showed the dimmed screen shrinking away after every Escape and every
// capture. Where a compositor blends, the overlay now draws a cleared
// frame and is withdrawn once that frame is on the window; without one
// a transparent pixel shows black, and the overlay is withdrawn on its
// last drawn frame. The class closed here is an overlay withdrawn with
// anything drawn where a compositor runs, a park that never withdraws,
// and a black frame where none runs. Not covered: a park from inside a
// draw (a failed save), whose cleared frame is the one drawing; a
// session re-armed between the cleared frame and the withdrawal, which
// the park's token leaves mapped; the withdrawal at the deadline when no
// present report comes; a real compositor's animation.

#[test]
fn a_parked_overlay_leaves_on_a_cleared_frame_where_a_compositor_blends() {
    last_frame(
        "a_parked_overlay_leaves_on_a_cleared_frame_where_a_compositor_blends",
        true,
    );
}

#[test]
fn a_parked_overlay_leaves_on_its_last_drawn_frame_where_no_compositor_runs() {
    last_frame(
        "a_parked_overlay_leaves_on_its_last_drawn_frame_where_no_compositor_runs",
        false,
    );
}

/// A daemon on a private Xvfb whose desktop is `DESKTOP`, under the
/// stand-in compositor with `composited`, its warm overlay parked.
/// Fields drop in order: the daemon first, then its state directory,
/// the X server last.
struct Desk {
    daemon: Daemon,
    _dir: tempfile::TempDir,
    _compositor: Option<Compositor>,
    conn: RustConnection,
    root: u32,
    /// The warm overlay window.
    warm: u32,
    _xvfb: daemon::Server,
}

fn desk(case: &str, composited: bool) -> Option<Desk> {
    if !enabled() {
        return None;
    }
    let (xvfb, display) = xvfb(case)?;
    let (conn, screen) = x11rb::connect(Some(&display)).unwrap();
    let root = conn.setup().roots[screen].root;
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().background_pixel(DESKTOP),
    )
    .unwrap();
    conn.clear_area(false, root, 0, 0, 0, 0).unwrap();
    let selection = conn
        .intern_atom(false, format!("_NET_WM_CM_S{screen}").as_bytes())
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let compositor = composited.then(|| {
        // A compositor draws each window from a pixmap of its own, which
        // keeps the window's last frame once the window unmaps.
        conn.composite_query_version(0, 4).unwrap().reply().unwrap();
        conn.composite_redirect_subwindows(root, Redirect::MANUAL)
            .unwrap();
        Compositor::start(&conn, root, selection)
    });
    let dir = tempfile::tempdir().unwrap();
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display).env_remove("WAYLAND_DISPLAY");
    });
    let warm = daemon.until("the warmup to open the overlay", || {
        all_windows_of(&conn, root, OVERLAY).first().copied()
    });
    daemon.settle();
    daemon.until("the warmup to park the overlay", || {
        windows_of(&conn, root, OVERLAY).is_empty().then_some(())
    });
    Some(Desk {
        daemon,
        _dir: dir,
        _compositor: compositor,
        conn,
        root,
        warm,
        _xvfb: xvfb,
    })
}

/// The overlay's pixel at (100, 100) as [b, g, r, a]; None while it is
/// unmapped.
fn overlay_pixel(conn: &RustConnection, warm: u32) -> Option<[u8; 4]> {
    conn.get_image(ImageFormat::Z_PIXMAP, warm, 100, 100, 1, 1, !0)
        .unwrap()
        .reply()
        .ok()
        .map(|image| <[u8; 4]>::try_from(&image.data[..4]).unwrap())
}

/// Whether `p` is the frozen screen under the dim: opaque, and darker
/// than the desktop. An unpainted window reads as the desktop, and the
/// dim drawn before the frame lands is translucent.
fn frozen_screen(p: &[u8; 4]) -> bool {
    p[3] == 255 && p[..3] != DESKTOP.to_le_bytes()[..3] && p[..3] != [0, 0, 0]
}

/// Cancel a capture and check what the overlay draws before it is
/// withdrawn: with `composited` it is withdrawn on a transparent frame,
/// and without it on the frame the cancel found.
fn last_frame(case: &str, composited: bool) {
    let Some(desk) = desk(case, composited) else {
        return;
    };
    let (daemon, conn, root, warm) = (&desk.daemon, &desk.conn, desk.root, desk.warm);
    daemon.send(&["--capture"]);
    let frozen = daemon.until("the capture's frozen screen to show", || {
        overlay_pixel(conn, warm).filter(|p| frozen_screen(p) && focus(conn) == warm)
    });
    // What the cancel draws: the window's damage until it unmaps, and,
    // where the stand-in compositor holds the window's pixmap, the
    // frame the window was withdrawn on, which a compositor animates.
    conn.change_window_attributes(
        warm,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::STRUCTURE_NOTIFY),
    )
    .unwrap();
    conn.damage_query_version(1, 1).unwrap().reply().unwrap();
    let damage = conn.generate_id().unwrap();
    conn.damage_create(damage, warm, ReportLevel::RAW_RECTANGLES)
        .unwrap();
    let pixmap = conn.generate_id().unwrap();
    // Past the session's entrance, whose frames are not the cancel's.
    daemon.settle();
    if composited {
        conn.composite_name_window_pixmap(warm, pixmap).unwrap();
    }
    conn.get_input_focus().unwrap().reply().unwrap();
    while conn.poll_for_event().unwrap().is_some() {}
    tap(conn, root, ESCAPE);
    let mut drawn = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match conn.poll_for_event().unwrap() {
            Some(Event::DamageNotify(e)) if e.drawable == warm => drawn += 1,
            Some(Event::UnmapNotify(e)) if e.window == warm => break,
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(1)),
        }
        assert!(
            Instant::now() < deadline,
            "the cancelled capture's overlay was not withdrawn within 5 s"
        );
    }
    if composited {
        let last = conn
            .get_image(ImageFormat::Z_PIXMAP, pixmap, 100, 100, 1, 1, !0)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            last.data[..4],
            [0, 0, 0, 0],
            "the overlay was withdrawn on a drawn frame, which a compositor \
             animates; the frozen screen read {frozen:?}"
        );
    } else {
        assert_eq!(
            drawn, 0,
            "the overlay drew after the cancel, where a transparent pixel \
             shows black"
        );
    }
}

// WHY: where no compositor blends, a translucent pixel shows over black.
// The re-armed overlay mapped at the press and drew its dim before the
// frozen screen landed, so a capture on such a screen began with a dark
// frame over every monitor for as long as the grab took, over 100 ms on
// a busy host. The overlay now maps there when its frame lands. The
// class closed here is any overlay frame before the frozen screen where
// no compositor runs, for a capture and for a record pick, which share
// the re-arm. Not covered: the fresh window a dead pool opens, which
// maps as it opens; the dim drawn at once where a compositor blends; a
// grab that never lands, which leaves the overlay unmapped.

#[test]
fn a_capture_overlay_maps_on_its_frozen_screen_where_no_compositor_runs() {
    let case = "a_capture_overlay_maps_on_its_frozen_screen_where_no_compositor_runs";
    let Some(desk) = desk(case, false) else {
        return;
    };
    let (daemon, conn, root, warm) = (&desk.daemon, &desk.conn, desk.root, desk.warm);
    for (session, command) in [
        ("capture 1", "--capture"),
        ("capture 2", "--capture"),
        ("the record pick", "--record-region"),
    ] {
        daemon.send(&[command]);
        // Each change of the overlay's pixel from the press to the
        // frozen screen: the desktop shows until then.
        let mut seen: Vec<[u8; 4]> = Vec::new();
        daemon.until(&format!("{session}'s frozen screen to show"), || {
            let p = overlay_pixel(conn, warm)?;
            if seen.last() != Some(&p) {
                seen.push(p);
            }
            frozen_screen(&p).then_some(())
        });
        let before: Vec<[u8; 4]> = seen[..seen.len() - 1]
            .iter()
            .filter(|p| p[..3] != DESKTOP.to_le_bytes()[..3])
            .copied()
            .collect();
        assert_eq!(
            before,
            Vec::<[u8; 4]>::new(),
            "{session}'s overlay drew before its frozen screen, which \
             shows over black where no compositor runs; its pixel went \
             {seen:?}"
        );
        tap(conn, root, ESCAPE);
        daemon.until(&format!("{session}'s overlay to park"), || {
            windows_of(conn, root, OVERLAY).is_empty().then_some(())
        });
    }
}

/// The desktop's colour, 0xRRGGBB, where the frozen screen shows.
const DESKTOP: u32 = 0x5a_7f_a6;
