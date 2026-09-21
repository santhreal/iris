use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use x11rb::connection::Connection;
#[cfg(target_os = "linux")]
use x11rb::protocol::xfixes::{ConnectionExt as XfixesExt, SelectionEventMask};
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as XprotoExt, CreateWindowAux, EventMask, PropMode,
    SelectionNotifyEvent, WindowClass, SELECTION_NOTIFY_EVENT,
};
#[cfg(target_os = "linux")]
use x11rb::protocol::Event;
#[cfg(target_os = "linux")]
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
#[cfg(target_os = "linux")]
use x11rb::CURRENT_TIME;

/// Validate and normalize drag paths before any platform work.
#[cfg(target_os = "linux")]
fn validate_drag_paths(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("no paths provided for file drag".to_string());
    }
    let mut path_bufs = Vec::with_capacity(paths.len());
    for pb in paths {
        if !pb.exists() {
            return Err(format!("drag path does not exist: {}", pb.display()));
        }
        path_bufs.push(pb);
    }
    Ok(path_bufs)
}

/// Drag-out: speaks the XDnD protocol directly from a 1x1 window.
/// Grab-less by design: pointer motion is tracked by polling
/// XQueryPointer, so a window manager's passive-grab activation on the
/// press that started the gesture cannot block the drag (GTK's
/// drag_begin fails with AlreadyGrabbed in that state). Selection
/// requests and XdndStatus/XdndFinished arrive as events on our own
/// connection, which grabs do not affect.
/// Thumbnail carried under the pointer during a drag: an
/// override-redirect window with a rounded shape mask, moved at poll
/// rate by the drag thread. Plain data; defined on every platform so
/// the non-Linux drag stub keeps the same signature.
pub struct DragIcon {
    /// Shared so a surface that already holds the pixels (the toast's
    /// thumb_rgba) hands over a refcount, not a multi-MB clone.
    pub rgba: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
}

/// `text/uri-list` payload for a set of paths: one `file://` URI per
/// line, CRLF-terminated per RFC 2483. Pure so the wire format is
/// testable without an X connection.
#[cfg(target_os = "linux")]
fn build_uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for pb in paths {
        let abs = std::fs::canonicalize(pb).unwrap_or_else(|_| pb.clone());
        out.push_str(&format!("file://{}\r\n", uri_encode_path(&abs.to_string_lossy())));
    }
    out
}

/// Percent-encode a filesystem path for a file:// URI: unreserved
/// characters and '/' pass through, everything else (spaces, UTF-8,
/// '%' itself) is %XX. Without this a path with a space produces a
/// URI every file manager rejects.
pub fn uri_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
#[cfg(target_os = "linux")]
pub fn start_file_drag_at_cursor(
    paths: Vec<PathBuf>,
    icon: Option<DragIcon>,
) -> Result<(), String> {
    let paths = validate_drag_paths(paths)?;
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return Err("file drag needs an X11 session".to_string());
    }
    let uri_list = build_uri_list(&paths);
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = XdndAtoms::intern(&conn)?;

    let win = conn
        .generate_id()
        .map_err(|e| format!("generate window id: {e}"))?;
    let aux = CreateWindowAux::new().override_redirect(1);
    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        win,
        root,
        -1,
        -1,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .map_err(|e| format!("create drag window: {e}"))?;
    conn.set_selection_owner(win, atoms.selection, CURRENT_TIME)
        .map_err(|e| format!("set XdndSelection owner: {e}"))?;
    let owner = conn
        .get_selection_owner(atoms.selection)
        .map_err(|e| format!("get XdndSelection owner: {e}"))?
        .reply()
        .map_err(|e| format!("get XdndSelection owner reply: {e}"))?
        .owner;
    if owner != win {
        return Err("failed to acquire XdndSelection".to_string());
    }
    let _ = conn.flush();

    std::thread::Builder::new()
        .name("iris-xdnd-drag".into())
        .spawn(move || run_xdnd_drag(conn, win, root, atoms, uri_list, icon))
        .map_err(|e| format!("spawn drag thread: {e}"))?;
    Ok(())
}

/// The pointer-following icon window: opaque 24-bit with the thumbnail
/// as its background pixmap and a 1-bit rounded-rect shape mask.
#[cfg(target_os = "linux")]
struct IconWindow {
    win: x11rb::protocol::xproto::Window,
    width: i16,
    height: i16,
}

#[cfg(target_os = "linux")]
impl IconWindow {
    fn show<C: Connection>(
        conn: &C,
        root: x11rb::protocol::xproto::Window,
        icon: DragIcon,
    ) -> Option<Self> {
        use x11rb::protocol::xproto::{Gcontext, ImageFormat, Pixmap};
        const MAX_W: u32 = 96;
        let (mut w, mut h) = (icon.width, icon.height);
        // The resize output owns its buffer; the unresized path keeps
        // borrowing the Arc. Cow so the borrow and the owned resize
        // share one binding.
        let data: std::borrow::Cow<[u8]> = if w > MAX_W {
            let nh = (h as u64 * MAX_W as u64 / w as u64).max(1) as u32;
            let img: image::ImageBuffer<image::Rgba<u8>, _> =
                image::ImageBuffer::from_raw(w, h, &icon.rgba[..])?;
            w = MAX_W;
            h = nh;
            std::borrow::Cow::Owned(
                image::imageops::resize(
                    &img,
                    MAX_W,
                    nh,
                    image::imageops::FilterType::Triangle,
                )
                .into_raw(),
            )
        } else {
            std::borrow::Cow::Borrowed(&icon.rgba[..])
        };
        let data: &[u8] = &data;
        let depth = conn.setup().roots[0].root_depth;
        let lsb = conn.setup().image_byte_order
            == x11rb::protocol::xproto::ImageOrder::LSB_FIRST;
        // ZPixmap at depth 24 is 4 bytes per pixel; LSBFirst wants B,G,R.
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for px in data.chunks_exact(4) {
            if lsb {
                pixels.extend_from_slice(&[px[2], px[1], px[0], 0xff]);
            } else {
                pixels.extend_from_slice(&[0xff, px[0], px[1], px[2]]);
            }
        }
        let pixmap: Pixmap = conn.generate_id().ok()?;
        let gc: Gcontext = conn.generate_id().ok()?;
        let mask: Pixmap = conn.generate_id().ok()?;
        let mgc: Gcontext = conn.generate_id().ok()?;
        let win = conn.generate_id().ok()?;
        // Every request issues before the first check: the server
        // runs them in order on one connection, so a serial check()
        // per request was a round trip each (~15 per drag start).
        // The cookies are checked in issue order after one flush; a
        // failure still reports against the request that caused it,
        // and dependent requests failing behind it change nothing.
        let c_pixmap = conn
            .create_pixmap(depth, pixmap, root, w as u16, h as u16)
            .ok()?;
        let c_gc = conn
            .create_gc(gc, root, &x11rb::protocol::xproto::CreateGCAux::new())
            .ok()?;
        let c_put = conn.put_image(
            ImageFormat::Z_PIXMAP,
            pixmap,
            gc,
            w as u16,
            h as u16,
            0,
            0,
            0,
            depth,
            &pixels,
        )
        .ok()?;
        // 1-bit mask: two rectangles plus four corner arcs = rounded rect.
        let r: i16 = 10;
        let (w16, h16) = (w as i16, h as i16);
        let c_mask = conn
            .create_pixmap(1, mask, root, w as u16, h as u16)
            .ok()?;
        let c_mgc = conn
            .create_gc(
                mgc,
                mask,
                &x11rb::protocol::xproto::CreateGCAux::new().foreground(0),
            )
            .ok()?;
        let c_clear = conn.poly_fill_rectangle(
            mask,
            mgc,
            &[x11rb::protocol::xproto::Rectangle {
                x: 0,
                y: 0,
                width: w as u16,
                height: h as u16,
            }],
        )
        .ok()?;
        let c_fg = conn
            .change_gc(
                mgc,
                &x11rb::protocol::xproto::ChangeGCAux::new().foreground(1),
            )
            .ok()?;
        use x11rb::protocol::xproto::{Arc, Rectangle};
        let rects = [
            Rectangle {
                x: r,
                y: 0,
                width: (w16 - 2 * r) as u16,
                height: h as u16,
            },
            Rectangle {
                x: 0,
                y: r,
                width: w as u16,
                height: (h16 - 2 * r) as u16,
            },
        ];
        let c_rects = conn.poly_fill_rectangle(mask, mgc, &rects).ok()?;
        let arc = |x: i16, y: i16, start: i16| Arc {
            x,
            y,
            width: (2 * r) as u16,
            height: (2 * r) as u16,
            angle1: start,
            angle2: 360 * 64,
        };
        let arcs = [
            arc(0, 0, 90 * 64),
            arc(w16 - 2 * r, 0, 0),
            arc(0, h16 - 2 * r, 180 * 64),
            arc(w16 - 2 * r, h16 - 2 * r, 270 * 64),
        ];
        let c_arcs = conn.poly_fill_arc(mask, mgc, &arcs).ok()?;

        let aux = CreateWindowAux::new()
            .override_redirect(1)
            .background_pixmap(pixmap)
            .border_pixel(0);
        let c_win = conn.create_window(
            x11rb::COPY_FROM_PARENT as u8,
            win,
            root,
            -100,
            -100,
            w as u16,
            h as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &aux,
        )
        .ok()?;
        use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
        let c_shape = conn
            .shape_mask(SO::SET, SK::BOUNDING, win, 0, 0, mask)
            .ok()?;
        let c_map = conn.map_window(win).ok()?;
        let c_fgc = conn.free_gc(gc).ok()?;
        let c_fmgc = conn.free_gc(mgc).ok()?;
        let c_fmask = conn.free_pixmap(mask).ok()?;
        // The window's background_pixmap attribute holds its own
        // server-side reference; ours is freed or it leaks per drag.
        let c_fpix = conn.free_pixmap(pixmap).ok()?;
        let _ = conn.flush();
        for c in [
            c_pixmap, c_gc, c_put, c_mask, c_mgc, c_clear, c_fg, c_rects, c_arcs, c_win,
            c_shape, c_map, c_fgc, c_fmgc, c_fmask, c_fpix,
        ] {
            if c.check().is_err() {
                // Free whatever the prefix created; freeing a resource
                // that never existed is an ignored error.
                let _ = conn.destroy_window(win);
                let _ = conn.free_gc(gc);
                let _ = conn.free_gc(mgc);
                let _ = conn.free_pixmap(mask);
                let _ = conn.free_pixmap(pixmap);
                let _ = conn.flush();
                return None;
            }
        }
        Some(Self {
            win,
            width: w16,
            height: h16,
        })
    }

    fn follow<C: Connection>(&self, conn: &C, px_x: i32, px_y: i32) {
        // Parked fully above the pointer: the pointer must never be
        // inside the icon's rect, or query_pointer returns the icon
        // itself and the drop target resolves to nothing.
        let _ = conn.configure_window(
            self.win,
            &x11rb::protocol::xproto::ConfigureWindowAux::new()
                .x(px_x - i32::from(self.width) / 2)
                .y(px_y - i32::from(self.height) - 2),
        );
    }

    fn destroy<C: Connection>(&self, conn: &C) {
        let _ = conn.destroy_window(self.win);
        let _ = conn.flush();
    }
}

#[cfg(target_os = "linux")]
struct XdndAtoms {
    selection: x11rb::protocol::xproto::Atom,
    aware: x11rb::protocol::xproto::Atom,
    proxy: x11rb::protocol::xproto::Atom,
    enter: x11rb::protocol::xproto::Atom,
    position: x11rb::protocol::xproto::Atom,
    status: x11rb::protocol::xproto::Atom,
    leave: x11rb::protocol::xproto::Atom,
    drop: x11rb::protocol::xproto::Atom,
    finished: x11rb::protocol::xproto::Atom,
    action_copy: x11rb::protocol::xproto::Atom,
    uri_list: x11rb::protocol::xproto::Atom,
    targets: x11rb::protocol::xproto::Atom,
    utf8: x11rb::protocol::xproto::Atom,
}

/// The four atoms a clipboard serve answers on. Grouping them stops a
/// TARGETS/uri-list/UTF8 transposition at the call site from compiling
/// silently.
#[cfg(target_os = "linux")]
struct CbAtoms {
    clipboard: x11rb::protocol::xproto::Atom,
    targets: x11rb::protocol::xproto::Atom,
    uri_list: x11rb::protocol::xproto::Atom,
    utf8: x11rb::protocol::xproto::Atom,
}

/// Interned atoms shared across drags and clipboard serves: atoms are
/// server-global constants, so the 13-name XDnD set and the 4-name
/// clipboard set resolve once per process instead of once per call.
#[cfg(target_os = "linux")]
fn atom_cached<C: Connection>(conn: &C, name: &'static [u8]) -> Option<u32> {
    static CACHE: std::sync::LazyLock<
        parking_lot::Mutex<std::collections::HashMap<&'static [u8], u32>>,
    > = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));
    if let Some(atom) = CACHE.lock().get(name) {
        return Some(*atom);
    }
    let atom = conn.intern_atom(false, name).ok()?.reply().ok()?.atom;
    CACHE.lock().insert(name, atom);
    Some(atom)
}

impl XdndAtoms {
    fn intern<C: Connection>(conn: &C) -> Result<Self, String> {
        let get = |name: &'static [u8]| {
            atom_cached(conn, name)
                .ok_or_else(|| format!("intern {} failed", String::from_utf8_lossy(name)))
        };
        Ok(Self {
            selection: get(b"XdndSelection")?,
            aware: get(b"XdndAware")?,
            proxy: get(b"XdndProxy")?,
            enter: get(b"XdndEnter")?,
            position: get(b"XdndPosition")?,
            status: get(b"XdndStatus")?,
            leave: get(b"XdndLeave")?,
            drop: get(b"XdndDrop")?,
            finished: get(b"XdndFinished")?,
            action_copy: get(b"XdndActionCopy")?,
            uri_list: get(b"text/uri-list")?,
            targets: get(b"TARGETS")?,
            utf8: get(b"UTF8_STRING")?,
        })
    }
}

/// Deepest XdndAware window containing (x, y) in root coordinates, and
/// its protocol version. `aware_cache` memoizes per-window awareness:
/// the drag polls this every 16ms and a get_property per window per
/// poll is a round-trip storm over a busy desktop.
#[cfg(target_os = "linux")]
fn xdnd_target_at<C: Connection>(
    conn: &C,
    win: x11rb::protocol::xproto::Window,
    first_child: x11rb::protocol::xproto::Window,
    aware: x11rb::protocol::xproto::Atom,
    proxy: x11rb::protocol::xproto::Atom,
    skip: Option<x11rb::protocol::xproto::Window>,
    aware_cache: &mut std::collections::HashMap<
        x11rb::protocol::xproto::Window,
        Option<(x11rb::protocol::xproto::Window, u32)>,
    >,
) -> Option<(x11rb::protocol::xproto::Window, u32)> {
    // The chain from `win` to the deepest window under the pointer:
    // query_pointer reports the immediate child containing the pointer,
    // so one round trip per level replaces a query_tree plus a
    // geometry+attributes pair per sibling. Unmapped windows cannot
    // contain the pointer, so no map-state check is needed. The
    // caller's own query_pointer on `win` supplies the first link:
    // re-asking it here was a duplicate round trip per pointer move.
    let mut chain = vec![win];
    let mut cur = first_child;
    if cur != x11rb::NONE && Some(cur) != skip {
        chain.push(cur);
    }
    for _ in 0..32 {
        if cur == x11rb::NONE || Some(cur) == skip {
            break;
        }
        let Some(ptr) = conn.query_pointer(cur).ok().and_then(|c| c.reply().ok()) else {
            break;
        };
        let child = ptr.child;
        if child == x11rb::NONE || Some(child) == skip {
            break;
        }
        chain.push(child);
        cur = child;
    }

    // XDnD targets the deepest XdndAware window on the chain. Awareness
    // is cached per window across the 16ms drag polls.
    let aware_of = |conn: &C,
                    w: x11rb::protocol::xproto::Window,
                    cache: &mut std::collections::HashMap<
        x11rb::protocol::xproto::Window,
        Option<(x11rb::protocol::xproto::Window, u32)>,
    >|
     -> Option<(x11rb::protocol::xproto::Window, u32)> {
        *cache.entry(w).or_insert_with(|| {
            let mut t = None;
            if let Some(reply) = conn
                .get_property(false, w, proxy, AtomEnum::ANY, 0, 2)
                .ok()
                .and_then(|c| c.reply().ok())
            {
                if let Some(mut it) = reply.value32() {
                    if let (Some(proxy_win), Some(version)) = (it.next(), it.next()) {
                        t = Some((proxy_win, version.min(5)));
                    }
                }
            }
            if t.is_none() {
                if let Some(reply) = conn
                    .get_property(false, w, aware, AtomEnum::ANY, 0, 1)
                    .ok()
                    .and_then(|c| c.reply().ok())
                {
                    if let Some(version) = reply.value32().and_then(|mut it| it.next()) {
                        t = Some((w, version.min(5)));
                    }
                }
            }
            t
        })
    };

    for w in chain.iter().rev() {
        if let Some(target) = aware_of(conn, *w, aware_cache) {
            return Some(target);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn send_client<C: Connection>(
    conn: &C,
    target: x11rb::protocol::xproto::Window,
    type_: x11rb::protocol::xproto::Atom,
    data: [u32; 5],
) {
    use x11rb::protocol::xproto::ClientMessageEvent;
    let _ = conn.send_event(
        false,
        target,
        EventMask::NO_EVENT,
        ClientMessageEvent::new(32, target, type_, data),
    );
}

/// Sleep until the connection has an event or `ms` elapse: the drag
/// loop's pointer cadence stays 16ms, but XdndFinished and
/// SelectionRequest wake it the instant they land.
#[cfg(target_os = "linux")]
fn wait_event_or(conn: &x11rb::rust_connection::RustConnection, ms: i32) {
    use std::os::unix::io::AsRawFd;
    let mut pfd = libc::pollfd {
        fd: conn.stream().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe {
        libc::poll(&mut pfd, 1, ms);
    }
}

/// XDnD source state machine. Runs until the drop completes, the drag is
/// cancelled, or the safety deadline passes.
#[cfg(target_os = "linux")]
fn run_xdnd_drag(
    conn: x11rb::rust_connection::RustConnection,
    win: x11rb::protocol::xproto::Window,
    root: x11rb::protocol::xproto::Window,
    atoms: XdndAtoms,
    uri_list: String,
    icon: Option<DragIcon>,
) {
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut target: Option<x11rb::protocol::xproto::Window> = None;
    let mut accepted = false;
    let mut dropped = false;
    let mut last_pos = (i32::MIN, i32::MIN);
    let mut last_icon_pos = (i32::MIN, i32::MIN);
    let mut aware_cache = std::collections::HashMap::new();
    let icon = icon.and_then(|icon| IconWindow::show(&conn, root, icon));

    while Instant::now() < deadline {
        while let Ok(Some(event)) = conn.poll_for_event() {
            match event {
                Event::ClientMessage(ev) => {
                    if ev.type_ == atoms.status {
                        // XDND: bit 0 of l[1] is the accept flag; bit 1 only
                        // asks for idle position updates.
                        accepted = ev.data.as_data32()[1] & 1 != 0;
                    } else if ev.type_ == atoms.finished {
                        // l[0] is the accepted flag: a refused drop
                        // still ends the drag, but the log distinguishes
                        // "target rejected" from a real copy.
                        let ok = ev.data.as_data32()[0] != 0;
                        crate::ilog!("iris: dragcopy: XdndFinished accepted={ok}");
                        if let Some(icon) = &icon {
                            icon.destroy(&conn);
                        }
                        let _ = conn.destroy_window(win);
                        let _ = conn.flush();
                        return;
                    }
                }
                Event::SelectionRequest(ev) => {
                    if ev.selection == atoms.selection {
                        serve_drag_request(&conn, &atoms, &uri_list, &ev);
                    }
                }
                Event::SelectionClear(ev) if ev.selection == atoms.selection => {
                    if let Some(icon) = &icon {
                        icon.destroy(&conn);
                    }
                    let _ = conn.destroy_window(win);
                    let _ = conn.flush();
                    return;
                }
                _ => {}
            }
        }

        let Some(pointer) = conn.query_pointer(root).ok().and_then(|c| c.reply().ok()) else {
            break;
        };
        let held = pointer
            .mask
            .contains(x11rb::protocol::xproto::KeyButMask::BUTTON1);
        let (px, py) = (i32::from(pointer.root_x), i32::from(pointer.root_y));

        if !held {
            if dropped {
                // Drop sent: stay alive until XdndFinished (or the
                // deadline) so the target can convert the selection.
                // poll() wakes the instant the reply lands instead of
                // up to 16ms late.
                wait_event_or(&conn, 16);
                continue;
            }
            if let Some(icon) = &icon {
                icon.destroy(&conn);
            }
            if let Some(t) = target.take() {
                if accepted {
                    send_client(&conn, t, atoms.drop, [win, 0, CURRENT_TIME, 0, 0]);
                    dropped = true;
                    let _ = conn.flush();
                    continue;
                }
                send_client(&conn, t, atoms.leave, [win, 0, 0, 0, 0]);
            }
            break;
        }
        // A stationary pointer cannot change the target chain: skip
        // the per-level query_pointer walk (a round trip each) and
        // the position resend. The 16ms cadence still polls for the
        // button release.
        if (px, py) != last_pos {
            let under = xdnd_target_at(
                &conn,
                root,
                pointer.child,
                atoms.aware,
                atoms.proxy,
                icon.as_ref().map(|i| i.win),
                &mut aware_cache,
            );
            let under_win = under.map(|(w, _)| w);
            if under_win != target {
                if let Some(old) = target.take() {
                    send_client(&conn, old, atoms.leave, [win, 0, 0, 0, 0]);
                }
                accepted = false;
                if let Some((w, version)) = under {
                    send_client(
                        &conn,
                        w,
                        atoms.enter,
                        [win, version << 24, atoms.uri_list, x11rb::NONE, x11rb::NONE],
                    );
                    target = Some(w);
                }
            }
            if let Some(t) = target {
                send_client(
                    &conn,
                    t,
                    atoms.position,
                    [
                        win,
                        0,
                        ((px as u32) << 16) | (py as u32 & 0xffff),
                        CURRENT_TIME,
                        atoms.action_copy,
                    ],
                );
            }
            last_pos = (px, py);
        }
        if let Some(icon) = &icon {
            if (px, py) != last_icon_pos {
                last_icon_pos = (px, py);
                icon.follow(&conn, px, py);
            }
        }
        let _ = conn.flush();
        // Events (XdndFinished, SelectionRequest) wake the loop early;
        // the timeout keeps the 16ms pointer cadence.
        wait_event_or(&conn, 16);
    }

    if let Some(icon) = &icon {
        icon.destroy(&conn);
    }
    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

/// Answers a selection conversion for the in-flight XDnD payload.
#[cfg(target_os = "linux")]
fn serve_drag_request<C: Connection>(
    conn: &C,
    atoms: &XdndAtoms,
    uri_list: &str,
    ev: &x11rb::protocol::xproto::SelectionRequestEvent,
) {
    let property = if ev.property == x11rb::NONE {
        ev.target
    } else {
        ev.property
    };
    let satisfied = if ev.target == atoms.targets {
        conn.change_property32(
            PropMode::REPLACE,
            ev.requestor,
            property,
            AtomEnum::ATOM,
            &[atoms.targets, atoms.uri_list, atoms.utf8],
        )
        .is_ok()
    } else if ev.target == atoms.uri_list {
        conn.change_property8(
            PropMode::REPLACE,
            ev.requestor,
            property,
            atoms.uri_list,
            uri_list.as_bytes(),
        )
        .is_ok()
    } else if ev.target == atoms.utf8 {
        conn.change_property8(
            PropMode::REPLACE,
            ev.requestor,
            property,
            atoms.utf8,
            uri_list.as_bytes(),
        )
        .is_ok()
    } else {
        false
    };
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: ev.time,
        requestor: ev.requestor,
        selection: ev.selection,
        target: ev.target,
        property: if satisfied { property } else { x11rb::NONE },
    };
    let _ = conn.send_event(false, ev.requestor, EventMask::NO_EVENT, notify);
    let _ = conn.flush();
}

#[cfg(not(target_os = "linux"))]
pub fn start_file_drag_at_cursor(
    _paths: Vec<PathBuf>,
    _icon: Option<DragIcon>,
) -> Result<(), String> {
    Err("file drag-out is implemented for Linux only".to_string())
}

/// Shared clipboard image set: used by the capture pipeline's
/// process-global clipboard owner.
pub fn clipboard_set_image(cb: &mut arboard::Clipboard, path: &std::path::Path) -> Result<(), String> {
    let img = image::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?
        .to_rgba8();
    cb.set_image(arboard::ImageData {
        width: img.width() as usize,
        height: img.height() as usize,
        bytes: std::borrow::Cow::Borrowed(img.as_raw()),
    })
    .map_err(|e| format!("clipboard set_image: {e}"))
}

pub fn clipboard_set_text(cb: &mut arboard::Clipboard, text: &str) -> Result<(), String> {
    cb.set_text(text)
        .map_err(|e| format!("clipboard set_text: {e}"))
}

/// Copy several files as one text/uri-list payload: file managers paste
/// the whole set, terminals receive the URI list.
pub fn copy_file_paths(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("no paths provided".to_string());
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = paths;
        Err("file copy needs an X11 session".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        let mut first = String::new();
        for pb in paths {
            if !pb.exists() {
                return Err(format!("file does not exist: {}", pb.display()));
            }
            if first.is_empty() {
                first = std::fs::canonicalize(pb)
                    .unwrap_or_else(|_| pb.clone())
                    .to_string_lossy()
                    .into_owned();
            }
        }
        serve_uri_list(build_uri_list(paths), first)
    }
}

/// Copy one file to the clipboard as text/uri-list (file managers paste
/// the file itself).
pub fn copy_file_path(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("file does not exist: {}", path.display()));
    }
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let abs_str = abs.to_string_lossy().into_owned();
    #[cfg(not(target_os = "linux"))]
    {
        let _ = abs_str;
        Err("file copy needs an X11 session".to_string())
    }
    #[cfg(target_os = "linux")]
    {
        serve_uri_list(format!("file://{}\r\n", uri_encode_path(&abs_str)), abs_str)
    }
}

/// Acquire CLIPBOARD with a text/uri-list payload and serve it from a
/// background thread until another owner takes over or 5 minutes pass.
#[cfg(target_os = "linux")]
pub fn serve_uri_list(uri_list: String, fallback_text: String) -> Result<(), String> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return Err("file copy needs an X11 session".to_string());
    }
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;

    let clipboard_atom = atom_cached(&conn, b"CLIPBOARD").ok_or("intern CLIPBOARD failed")?;
    let targets_atom = atom_cached(&conn, b"TARGETS").ok_or("intern TARGETS failed")?;
    let uri_list_atom =
        atom_cached(&conn, b"text/uri-list").ok_or("intern text/uri-list failed")?;
    let utf8_atom = atom_cached(&conn, b"UTF8_STRING").ok_or("intern UTF8_STRING failed")?;

    let win = conn
        .generate_id()
        .map_err(|e| format!("generate window id: {e}"))?;
    let aux = CreateWindowAux::new()
        .override_redirect(1)
        .event_mask(EventMask::PROPERTY_CHANGE);
    conn.create_window(
        x11rb::COPY_FROM_PARENT as u8,
        win,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &aux,
    )
    .map_err(|e| format!("create clipboard window: {e}"))?;

    conn.set_selection_owner(win, clipboard_atom, CURRENT_TIME)
        .map_err(|e| format!("set selection owner: {e}"))?;
    let owner = conn
        .get_selection_owner(clipboard_atom)
        .map_err(|e| format!("get selection owner: {e}"))?
        .reply()
        .map_err(|e| format!("get selection owner reply: {e}"))?
        .owner;
    if owner != win {
        return Err("failed to acquire CLIPBOARD selection".to_string());
    }

    if let Ok(cookie) = conn.xfixes_query_version(4, 0) {
        if cookie.reply().is_ok() {
            let _ = conn.xfixes_select_selection_input(
                root,
                clipboard_atom,
                SelectionEventMask::SET_SELECTION_OWNER,
            );
        }
    }
    let _ = conn.flush();

    std::thread::Builder::new()
        .name("iris-x11-clipboard".into())
        .spawn(move || {
            serve_x11_clipboard(
                conn,
                win,
                CbAtoms {
                    clipboard: clipboard_atom,
                    targets: targets_atom,
                    uri_list: uri_list_atom,
                    utf8: utf8_atom,
                },
                uri_list,
                fallback_text,
            );
        })
        .map_err(|e| format!("spawn clipboard thread: {e}"))?;
    Ok(())
}


#[cfg(target_os = "linux")]
fn serve_x11_clipboard(
    conn: x11rb::rust_connection::RustConnection,
    win: x11rb::protocol::xproto::Window,
    atoms: CbAtoms,
    uri_list: String,
    abs_str: String,
) {
    let (clipboard_atom, targets_atom, uri_list_atom, utf8_atom) =
        (atoms.clipboard, atoms.targets, atoms.uri_list, atoms.utf8);
    let deadline = Instant::now() + Duration::from_secs(300);
    // Event-driven: poll() the connection fd so a paste request is
    // answered the instant it arrives instead of up to a sleep
    // interval late, and an idle clipboard costs no wakeups.
    use std::os::unix::io::AsRawFd;
    let x_fd = conn.stream().as_raw_fd();
    while Instant::now() < deadline {
        let event = match conn.poll_for_event() {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let mut pfd = libc::pollfd {
                    fd: x_fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                unsafe {
                    libc::poll(&mut pfd, 1, remaining.as_millis().min(i32::MAX as u128) as i32);
                }
                continue;
            }
            Err(_) => break,
        };

        match event {
            Event::SelectionClear(ev) => {
                if ev.selection == clipboard_atom {
                    break;
                }
            }
            Event::XfixesSelectionNotify(ev) => {
                if ev.selection == clipboard_atom && ev.owner != win {
                    break;
                }
            }
            Event::SelectionRequest(ev) if ev.selection == clipboard_atom => {
                let property = if ev.property == x11rb::NONE {
                    ev.target
                } else {
                    ev.property
                };

                let mut satisfied = false;
                if ev.target == targets_atom {
                    let targets = [targets_atom, uri_list_atom, utf8_atom];
                    if conn
                        .change_property32(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            AtomEnum::ATOM,
                            &targets,
                        )
                        .is_ok()
                    {
                        satisfied = true;
                    }
                } else if ev.target == uri_list_atom {
                    if conn
                        .change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            uri_list_atom,
                            uri_list.as_bytes(),
                        )
                        .is_ok()
                    {
                        satisfied = true;
                    }
                } else if ev.target == utf8_atom
                    && conn
                        .change_property8(
                            PropMode::REPLACE,
                            ev.requestor,
                            property,
                            utf8_atom,
                            abs_str.as_bytes(),
                        )
                        .is_ok()
                {
                    satisfied = true;
                }

                let notify = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: ev.time,
                    requestor: ev.requestor,
                    selection: ev.selection,
                    target: ev.target,
                    property: if satisfied { property } else { x11rb::NONE },
                };

                let _ = conn.send_event(false, ev.requestor, EventMask::NO_EVENT, notify);
                let _ = conn.flush();
            }
            _ => {}
        }
    }

    let _ = conn.destroy_window(win);
    let _ = conn.flush();
}

// WHY: the class closed here is "a drag or copy starts with a path that
// cannot serve": an empty set or a vanished file must fail before any X11
// work begins, not mid-drag. Not covered: the XDnD wire protocol, which
// the on-rig QA scripts exercise against a live drop target.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_rejects_empty_and_missing() {
        assert!(validate_drag_paths(Vec::new()).is_err());
        assert!(validate_drag_paths(vec![PathBuf::from("/nonexistent-xyz")]).is_err());
    }

    #[test]
    fn uri_list_is_crlf_file_uris() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"y").unwrap();
        let out = build_uri_list(&[a.clone(), b.clone()]);
        // RFC 2483: one absolute file URI per line, CRLF terminated.
        let lines: Vec<&str> = out.split("\r\n").collect();
        assert_eq!(lines.len(), 3, "two URIs plus trailing empty after last CRLF");
        assert!(lines[0].starts_with("file://"));
        assert!(lines[0].ends_with("a.png"));
        assert!(lines[1].ends_with("b.png"));
        assert_eq!(lines[2], "");
    }

    #[test]
    fn validate_accepts_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.png");
        std::fs::write(&f, b"x").unwrap();
        let out = validate_drag_paths(vec![f.clone()]).unwrap();
        assert_eq!(out, vec![f]);
    }
}
