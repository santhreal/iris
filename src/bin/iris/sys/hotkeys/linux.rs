use futures::channel::mpsc::UnboundedSender;
use iris_lib::config::Config;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use crate::daemon::Command;

/// Keysym values for the hotkey grammar (X11 keysym numbers).
fn keysym_for(name: &str) -> Option<u32> {
    let lower = name.to_lowercase();
    Some(match lower.as_str() {
        "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" => 0xff61,
        "escape" | "esc" => 0xff1b,
        "enter" | "return" => 0xff0d,
        "space" | "spacebar" => 0x20,
        "tab" => 0xff09,
        "backspace" => 0xff08,
        "delete" | "del" => 0xffff,
        "home" => 0xff50,
        "end" => 0xff57,
        "pageup" | "pgup" => 0xff55,
        "pagedown" | "pgdn" => 0xff56,
        "up" | "arrowup" => 0xff52,
        "down" | "arrowdown" => 0xff54,
        "left" | "arrowleft" => 0xff51,
        "right" | "arrowright" => 0xff53,
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            match c {
                'a'..='z' => c as u32,
                '0'..='9' => c as u32,
                _ => return None,
            }
        }
        s if s.starts_with('f') => {
            let n: u32 = s[1..].parse().ok()?;
            if !(1..=24).contains(&n) {
                return None;
            }
            0xffbd + n
        }
        _ => return None,
    })
}

/// "Ctrl+Shift+R" -> (modifier mask, keysym).
fn parse_hotkey(s: &str) -> Option<(ModMask, u32)> {
    let mut mods = ModMask::from(0u16);
    let mut key = None;
    for tok in s.split('+').map(|t| t.trim()).filter(|t| !t.is_empty()) {
        match tok.to_lowercase().as_str() {
            "ctrl" | "control" => mods |= ModMask::CONTROL,
            "shift" => mods |= ModMask::SHIFT,
            "alt" | "option" => mods |= ModMask::M1,
            "super" | "cmd" | "command" | "meta" | "win" | "windows" => mods |= ModMask::M4,
            name => key = keysym_for(name),
        }
    }
    key.map(|k| (mods, k))
}

/// The full keyboard mapping, fetched once per regrab: resolving
/// each hotkey's keycode against a fresh reply was a round trip
/// per key.
struct Keymap {
    lo: u8,
    per: usize,
    keysyms: Vec<u32>,
}

impl Keymap {
    fn fetch(conn: &impl Connection) -> Option<Self> {
        let setup = conn.setup();
        let reply = conn
            .get_keyboard_mapping(setup.min_keycode, setup.max_keycode - setup.min_keycode + 1)
            .ok()?
            .reply()
            .ok()?;
        Some(Self {
            lo: setup.min_keycode,
            per: reply.keysyms_per_keycode as usize,
            keysyms: reply.keysyms,
        })
    }
    /// First keycode producing `keysym` at any shift level.
    fn keycode_for(&self, keysym: u32) -> Option<u8> {
        for (i, chunk) in self.keysyms.chunks(self.per).enumerate() {
            if chunk.contains(&keysym) {
                return Some(self.lo + i as u8);
            }
        }
        None
    }
}

/// Signalled by settings save: ungrab everything, reload config,
/// re-grab. The grab thread owns the connection. The signal is a
/// byte on a self-pipe so the grab thread's poll() wakes on it
/// instead of timing out every 50ms to check a channel.
static REGRAB_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

pub(super) fn request_regrab() {
    let fd = REGRAB_FD.load(std::sync::atomic::Ordering::Acquire);
    if fd >= 0 {
        let byte = [1u8];
        unsafe {
            libc::write(fd, byte.as_ptr() as *const libc::c_void, 1);
        }
    }
}

/// (keycode, modifier mask, command) per grabbed chord: the mask
/// matters when two hotkeys share a key, e.g. "R" and "Ctrl+R".
type Grabs = Vec<(u8, ModMask, Command)>;

/// Replace every key grab `conn` holds on `root` with the configured
/// hotkeys, each with the four NumLock/CapsLock variants.
fn grab_all(conn: &RustConnection, root: Window) -> Grabs {
    let locks = [
        ModMask::from(0u16),
        ModMask::M2,
        ModMask::LOCK,
        ModMask::M2 | ModMask::LOCK,
    ];
    let keymap = Keymap::fetch(conn);
    let _ = conn.ungrab_key(0u8, root, ModMask::from(0x8000u16));
    let mut grabbed = Grabs::new();
    let cfg = Config::load();
    for (hotkey, cmd) in [
        (&cfg.capture_hotkey, Command::Capture),
        (&cfg.record_hotkey, Command::RecordToggle),
    ] {
        let Some((mods, keysym)) = parse_hotkey(hotkey) else {
            iris_lib::ilog!("iris: cannot parse hotkey {hotkey:?}");
            continue;
        };
        let Some(keycode) = keymap.as_ref().and_then(|k| k.keycode_for(keysym)) else {
            iris_lib::ilog!("iris: no keycode for hotkey keysym {keysym:#x}");
            continue;
        };
        // Issue all four lock-variant grabs before checking any: a
        // serial check() per grab is a round trip each, four per
        // hotkey per regrab.
        let cookies: Vec<_> = locks
            .iter()
            .map(|extra| {
                conn.grab_key(
                    false,
                    root,
                    mods | *extra,
                    keycode,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
            })
            .collect();
        let ok = cookies.into_iter().all(|c| {
            c.map_err(|e| e.to_string())
                .and_then(|cookie| cookie.check().map_err(|e| e.to_string()))
                .is_ok()
        });
        if ok {
            grabbed.push((keycode, mods, cmd));
        } else {
            iris_lib::ilog!("iris: cannot grab hotkey keysym {keysym:#x}");
        }
    }
    let _ = conn.flush();
    grabbed
}

/// Grab the configured hotkeys on the root window, then forward presses
/// from a thread of their own. The grabs are in place when this
/// returns: the daemon binds its socket after it, so a hotkey pressed
/// once the socket accepts reaches the grab. Off an X11 session this
/// does nothing: a Wayland session's `DISPLAY` is XWayland, which
/// delivers keys only while an X11 client has the focus, and there the
/// compositor binds `iris --capture` instead.
pub(super) fn spawn(tx: UnboundedSender<Command>) {
    if !iris_lib::session::x11() {
        return;
    }
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        iris_lib::ilog!("iris: hotkeys: cannot connect to the X server");
        return;
    };
    // Self-pipe: the read end joins the X fd in poll(), the write end
    // is published for request_regrab.
    let mut pipe_fds = [-1i32; 2];
    if unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        iris_lib::ilog!("iris: hotkeys: {}", std::io::Error::last_os_error());
        return;
    }
    let root = conn.setup().roots[screen_num].root;
    let mut grabbed = grab_all(&conn, root);
    REGRAB_FD.store(pipe_fds[1], std::sync::atomic::Ordering::Release);
    std::thread::spawn(move || {
        // Event-driven, not polled: poll() the connection's fd and
        // the regrab pipe, so a grabbed keypress or a settings save
        // wakes the thread the instant it lands. No timeout: the
        // thread sleeps until one of the two fds is readable.
        use std::os::unix::io::AsRawFd;
        let x_fd = conn.stream().as_raw_fd();
        let mut pfds = [
            libc::pollfd {
                fd: x_fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: pipe_fds[0],
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        loop {
            // Drain every queued event before sleeping again.
            loop {
                match conn.poll_for_event() {
                    Ok(Some(x11rb::protocol::Event::KeyPress(ev))) => {
                        // Match keycode AND modifier mask: two
                        // hotkeys can share a key ("R" vs "Ctrl+R")
                        // and the grab fires per (key, mods) pair.
                        // Lock bits (NumLock M2, CapsLock LOCK) are
                        // masked out: they are grabbed as variants
                        // but are not part of the configured mask.
                        let ev_state: u16 = ev.state.into();
                        let ev_mods = ModMask::from(
                            ev_state & !(u16::from(ModMask::M2) | u16::from(ModMask::LOCK)),
                        );
                        if let Some((_, _, cmd)) = grabbed
                            .iter()
                            .find(|(kc, m, _)| *kc == ev.detail && *m == ev_mods)
                        {
                            if tx.unbounded_send(cmd.clone()).is_err() {
                                return;
                            }
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
            for p in &mut pfds {
                p.revents = 0;
            }
            if unsafe { libc::poll(pfds.as_mut_ptr(), 2, -1) } <= 0 {
                continue;
            }
            if pfds[1].revents & libc::POLLIN != 0 {
                // Drain the pipe, then regrab once per wake.
                let mut buf = [0u8; 64];
                unsafe {
                    libc::read(
                        pipe_fds[0],
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    );
                }
                grabbed = grab_all(&conn, root);
            }
        }
    });
}
