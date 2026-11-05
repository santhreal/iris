use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

use crate::capture::WinRect;

/// A process-shared X connection for read-only queries (monitors,
/// window list). The handshake is ~10ms; a daemon that opens one per
/// capture, toast, and editor pays it on every surface. The connection
/// is Send+Sync and read-only here, so one serves every caller.
pub fn shared_conn() -> Result<(&'static x11rb::rust_connection::RustConnection, usize), String> {
    static CONN: std::sync::LazyLock<
        Result<(x11rb::rust_connection::RustConnection, usize), String>,
    > = std::sync::LazyLock::new(|| x11rb::connect(None).map_err(|e| format!("X11 connect: {e}")));
    CONN.as_ref().map(|(c, s)| (c, *s)).map_err(|e| e.clone())
}

/// Interned atoms shared across every query: intern_atom is a round
/// trip, and the same EWMH names resolve on every capture and window
/// probe.
fn atom_cached(conn: &impl Connection, name: &'static [u8]) -> Option<u32> {
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

/// Active monitor rectangles in root (frame) pixels, primary first.
/// Falls back to the whole root when randr reports no monitors.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    let (conn, screen_num) = shared_conn()?;
    let screen = &conn.setup().roots[screen_num];
    let reply = conn
        .randr_get_monitors(screen.root, true)
        .map_err(|e| format!("randr get_monitors: {e}"))?
        .reply()
        .map_err(|e| format!("randr get_monitors reply: {e}"))?;
    let mut primary = Vec::new();
    let mut rest = Vec::new();
    for m in &reply.monitors {
        let rect = WinRect {
            x: m.x as i32,
            y: m.y as i32,
            width: m.width as u32,
            height: m.height as u32,
        };
        if m.primary {
            primary.push(rect);
        } else {
            rest.push(rect);
        }
    }
    primary.extend(rest);
    if primary.is_empty() {
        primary.push(WinRect {
            x: 0,
            y: 0,
            width: screen.width_in_pixels as u32,
            height: screen.height_in_pixels as u32,
        });
    }
    Ok(primary)
}

/// `monitor` less the edges that panels and docks reserve: its overlap
/// with the window manager's `_NET_WORKAREA` for the current desktop
/// (EWMH). `monitor` itself where no window manager sets one, or where
/// they do not overlap.
pub fn work_area(monitor: WinRect) -> WinRect {
    net_workarea()
        .and_then(|area| monitor.intersect(&area))
        .unwrap_or(monitor)
}

/// The current desktop's `_NET_WORKAREA` rect, in root pixels.
fn net_workarea() -> Option<WinRect> {
    let (conn, screen_num) = shared_conn().ok()?;
    let root = conn.setup().roots[screen_num].root;
    let cardinals = |name: &'static [u8], offset: u32, length: u32| -> Option<Vec<u32>> {
        let atom = atom_cached(conn, name)?;
        let reply = conn
            .get_property(false, root, atom, AtomEnum::CARDINAL, offset, length)
            .ok()?
            .reply()
            .ok()?;
        let values = reply.value32()?.collect();
        Some(values)
    };
    let desktop = cardinals(b"_NET_CURRENT_DESKTOP", 0, 1)
        .and_then(|d| d.first().copied())
        .unwrap_or(0);
    let area = cardinals(b"_NET_WORKAREA", desktop.checked_mul(4)?, 4)?;
    let [x, y, width, height] = area[..] else {
        return None;
    };
    Some(WinRect {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        width,
        height,
    })
}

/// Monitors and snappable windows over one X connection: the capture
/// path queries both back to back, and a connection handshake is
/// tens of ms of dead latency before the overlay can open.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    let monitors = monitors()?;
    let (conn, screen_num) = shared_conn()?;
    let windows = list_top_level_windows_on(conn, screen_num)?;
    Ok((monitors, windows))
}

/// The pointer's position in root (frame) pixels.
pub fn pointer() -> Result<(i32, i32), String> {
    let (conn, screen_num) = shared_conn()?;
    let root = conn.setup().roots[screen_num].root;
    let reply = conn
        .query_pointer(root)
        .map_err(|e| format!("query_pointer: {e}"))?
        .reply()
        .map_err(|e| format!("query_pointer reply: {e}"))?;
    Ok((i32::from(reply.root_x), i32::from(reply.root_y)))
}

/// Top-level windows in bottom-to-top stacking order (EWMH
/// _NET_CLIENT_LIST_STACKING), minus this process's own windows (the
/// frozen-frame overlay itself) and iconic/hidden ones. The overlay uses
/// this for hover-snap: the last rect containing the cursor is the
/// topmost candidate.
pub fn list_top_level_windows() -> Result<Vec<WinRect>, String> {
    let (conn, screen_num) = shared_conn()?;
    list_top_level_windows_on(conn, screen_num)
}

fn list_top_level_windows_on(
    conn: &impl Connection,
    screen_num: usize,
) -> Result<Vec<WinRect>, String> {
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    // Cached atoms: the same names resolve on every capture.
    let stacking = atom_cached(conn, b"_NET_CLIENT_LIST_STACKING")
        .ok_or("intern _NET_CLIENT_LIST_STACKING failed")?;
    let wm_pid = atom_cached(conn, b"_NET_WM_PID").ok_or("intern _NET_WM_PID failed")?;
    let wm_state = atom_cached(conn, b"_NET_WM_STATE").ok_or("intern _NET_WM_STATE failed")?;
    let state_hidden =
        atom_cached(conn, b"_NET_WM_STATE_HIDDEN").ok_or("intern _NET_WM_STATE_HIDDEN failed")?;
    let (frame, margin) = extent_atoms(conn)?;

    let list = conn
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, u32::MAX)
        .map_err(|e| format!("query stacking: {e}"))?
        .reply()
        .map_err(|e| format!("query stacking reply: {e}"))?;
    let mut windows: Vec<u32> = list.value32().map(|v| v.collect()).unwrap_or_default();
    if windows.is_empty() {
        // A window manager that sets no stacking list, as xfwm4 4.18
        // with one window open, or none at all: the root's children, in
        // bottom-to-top stacking order. A reparenting window manager's
        // child is its frame, which has no _NET_WM_PID or extents, so
        // each child is read as the client inside it.
        let tree = conn
            .query_tree(root)
            .map_err(|e| format!("query_tree: {e}"))?
            .reply()
            .map_err(|e| format!("query_tree reply: {e}"))?;
        // Pipeline the attribute reads: a serial reply() per child is
        // a round trip per window on a WM-less session.
        let cookies: Vec<_> = tree
            .children
            .iter()
            .map(|win| (*win, conn.get_window_attributes(*win).ok()))
            .collect();
        let _ = conn.flush();
        for (win, cookie) in cookies {
            let viewable = cookie
                .and_then(|c| c.reply().ok())
                .map(|a| {
                    a.map_state == x11rb::protocol::xproto::MapState::VIEWABLE
                        && a.class == x11rb::protocol::xproto::WindowClass::INPUT_OUTPUT
                })
                .unwrap_or(false);
            if viewable {
                windows.push(win);
            }
        }
        windows = clients(conn, windows);
    }

    let own_pid = std::process::id();
    // Issue every per-window request before reading any reply: six
    // round trips per window serialized is a visible slice of the
    // overlay's open latency on a busy desktop.
    type Prop<'a, C> = x11rb::cookie::Cookie<'a, C, x11rb::protocol::xproto::GetPropertyReply>;
    struct Probe<'a, C: Connection> {
        pid: Option<Prop<'a, C>>,
        state: Option<Prop<'a, C>>,
        frame: Option<Prop<'a, C>>,
        margin: Option<Prop<'a, C>>,
        geom: Option<x11rb::cookie::Cookie<'a, C, x11rb::protocol::xproto::GetGeometryReply>>,
        trans: Option<
            x11rb::cookie::Cookie<'a, C, x11rb::protocol::xproto::TranslateCoordinatesReply>,
        >,
    }
    let mut probes = Vec::with_capacity(windows.len());
    for win in windows {
        probes.push(Probe {
            pid: conn
                .get_property(false, win, wm_pid, AtomEnum::CARDINAL, 0, 1)
                .ok(),
            state: conn
                .get_property(false, win, wm_state, AtomEnum::ATOM, 0, 32)
                .ok(),
            frame: conn
                .get_property(false, win, frame, AtomEnum::CARDINAL, 0, 4)
                .ok(),
            margin: conn
                .get_property(false, win, margin, AtomEnum::CARDINAL, 0, 4)
                .ok(),
            geom: conn.get_geometry(win).ok(),
            trans: conn.translate_coordinates(win, root, 0, 0).ok(),
        });
    }
    let _ = conn.flush();
    let mut rects = Vec::with_capacity(probes.len());
    for probe in probes {
        // Own windows (overlay, chips, borders) must never be snappable.
        if let Some(reply) = probe.pid.and_then(|c| c.reply().ok()) {
            if let Some(mut it) = reply.value32() {
                if it.next() == Some(own_pid) {
                    continue;
                }
            }
        }
        // Skip minimized windows.
        if let Some(reply) = probe.state.and_then(|c| c.reply().ok()) {
            if reply
                .value32()
                .map(|mut v| v.any(|s| s == state_hidden))
                .unwrap_or(false)
            {
                continue;
            }
        }
        let geom = match probe.geom.and_then(|c| c.reply().ok()) {
            Some(g) => g,
            None => continue, // window vanished mid-enumeration
        };
        let origin = match probe.trans.and_then(|c| c.reply().ok()) {
            Some(t) => t,
            None => continue,
        };
        if geom.width == 0 || geom.height == 0 {
            continue;
        }
        rects.push(shown_rect(
            client_rect(&geom, &origin),
            extents(probe.frame),
            extents(probe.margin),
        ));
    }
    Ok(rects)
}

/// How many levels below a top-level window [`clients`] searches: a
/// window manager's frame holds the client, or a wrapper that holds it.
const CLIENT_DEPTH: usize = 2;

/// The client window each of `tops` frames: the window itself, or the
/// first window within [`CLIENT_DEPTH`] levels below it, that has the
/// ICCCM `WM_STATE` a window manager sets on every client it manages. A
/// window with none below it is its own client, as with no window
/// manager. Each level's requests are sent before any reply is read.
fn clients(conn: &impl Connection, tops: Vec<u32>) -> Vec<u32> {
    let Some(wm_state) = atom_cached(conn, b"WM_STATE") else {
        return tops;
    };
    let mut found: Vec<Option<u32>> = vec![None; tops.len()];
    let mut level: Vec<(usize, u32)> = tops.iter().copied().enumerate().collect();
    for depth in 0..=CLIENT_DEPTH {
        let probes: Vec<_> = level
            .iter()
            .map(|&(top, win)| {
                let state = conn
                    .get_property(false, win, wm_state, AtomEnum::ANY, 0, 0)
                    .ok();
                let tree = if depth < CLIENT_DEPTH {
                    conn.query_tree(win).ok()
                } else {
                    None
                };
                (top, win, state, tree)
            })
            .collect();
        let _ = conn.flush();
        let mut next = Vec::new();
        for (top, win, state, tree) in probes {
            let managed = state
                .and_then(|c| c.reply().ok())
                .is_some_and(|r| r.type_ != x11rb::NONE);
            let tree = tree.and_then(|c| c.reply().ok());
            if found[top].is_some() {
                continue;
            }
            if managed {
                found[top] = Some(win);
            } else if let Some(tree) = tree {
                next.extend(tree.children.into_iter().map(|child| (top, child)));
            }
        }
        next.retain(|&(top, _)| found[top].is_none());
        if next.is_empty() {
            break;
        }
        level = next;
    }
    tops.into_iter()
        .zip(found)
        .map(|(top, client)| client.unwrap_or(top))
        .collect()
}

/// The focused top-level window's rect in root (frame) pixels, from
/// EWMH _NET_ACTIVE_WINDOW. Used by --capture-window: the shot is the
/// window as it shows on screen, frame included, with no overlay round
/// trip.
pub fn active_window_rect() -> Result<WinRect, String> {
    let (conn, screen_num) = shared_conn()?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let atom =
        atom_cached(conn, b"_NET_ACTIVE_WINDOW").ok_or("intern _NET_ACTIVE_WINDOW failed")?;
    let (frame, margin) = extent_atoms(conn)?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|e| format!("query active window: {e}"))?
        .reply()
        .map_err(|e| format!("query active window reply: {e}"))?;
    let win = reply
        .value32()
        .and_then(|mut v| v.next())
        .filter(|w| *w != x11rb::NONE)
        .ok_or("no active window")?;
    // The geometry, the root-space origin, and the frame extents are
    // independent: send every request before awaiting any.
    let geom_cookie = conn
        .get_geometry(win)
        .map_err(|e| format!("get_geometry: {e}"))?;
    let trans_cookie = conn
        .translate_coordinates(win, root, 0, 0)
        .map_err(|e| format!("translate_coordinates: {e}"))?;
    let frame = conn
        .get_property(false, win, frame, AtomEnum::CARDINAL, 0, 4)
        .ok();
    let margin = conn
        .get_property(false, win, margin, AtomEnum::CARDINAL, 0, 4)
        .ok();
    let geom = geom_cookie
        .reply()
        .map_err(|e| format!("get_geometry reply: {e}"))?;
    let origin = trans_cookie
        .reply()
        .map_err(|e| format!("translate_coordinates reply: {e}"))?;
    Ok(shown_rect(
        client_rect(&geom, &origin),
        extents(frame),
        extents(margin),
    ))
}

/// `_NET_FRAME_EXTENTS` and `_GTK_FRAME_EXTENTS`, in that order.
fn extent_atoms(conn: &impl Connection) -> Result<(u32, u32), String> {
    let frame =
        atom_cached(conn, b"_NET_FRAME_EXTENTS").ok_or("intern _NET_FRAME_EXTENTS failed")?;
    let margin =
        atom_cached(conn, b"_GTK_FRAME_EXTENTS").ok_or("intern _GTK_FRAME_EXTENTS failed")?;
    Ok((frame, margin))
}

/// An X window's rect in root pixels, its border included.
fn client_rect(
    geom: &x11rb::protocol::xproto::GetGeometryReply,
    origin: &x11rb::protocol::xproto::TranslateCoordinatesReply,
) -> WinRect {
    let border = i32::from(geom.border_width);
    WinRect {
        x: i32::from(origin.dst_x) - border,
        y: i32::from(origin.dst_y) - border,
        width: u32::from(geom.width) + 2 * u32::from(geom.border_width),
        height: u32::from(geom.height) + 2 * u32::from(geom.border_width),
    }
}

/// A four-CARDINAL extents property (left, right, top, bottom), or
/// zeros when the window does not set it.
fn extents<C: Connection>(
    cookie: Option<x11rb::cookie::Cookie<'_, C, x11rb::protocol::xproto::GetPropertyReply>>,
) -> [u32; 4] {
    cookie
        .and_then(|c| c.reply().ok())
        .and_then(|reply| {
            let mut v = reply.value32()?;
            Some([v.next()?, v.next()?, v.next()?, v.next()?])
        })
        .unwrap_or_default()
}

/// A window's rect as it shows on screen, from `client`, the X window
/// with its border. A reparenting window manager draws a frame (title
/// bar and borders) around the client and sets its size in
/// `_NET_FRAME_EXTENTS`: `frame` grows the rect. A client-side
/// decorated window pads itself with a transparent shadow and sets its
/// size in `_GTK_FRAME_EXTENTS`: `margin` shrinks the rect. Both are
/// left, right, top, bottom. A result with no area, or outside the
/// coordinate range, is `client`.
fn shown_rect(client: WinRect, frame: [u32; 4], margin: [u32; 4]) -> WinRect {
    let [fl, fr, ft, fb] = frame.map(i64::from);
    let [ml, mr, mt, mb] = margin.map(i64::from);
    let x = i64::from(client.x) - fl + ml;
    let y = i64::from(client.y) - ft + mt;
    let width = i64::from(client.width) + fl + fr - ml - mr;
    let height = i64::from(client.height) + ft + fb - mt - mb;
    if width <= 0 || height <= 0 {
        return client;
    }
    match (
        i32::try_from(x),
        i32::try_from(y),
        u32::try_from(width),
        u32::try_from(height),
    ) {
        (Ok(x), Ok(y), Ok(width), Ok(height)) => WinRect {
            x,
            y,
            width,
            height,
        },
        _ => client,
    }
}

// WHY: the class closed here is "a window capture is not the window as
// it shows": the hover snap and --capture-window used the client
// window, which leaves out a window manager's title bar and borders,
// and takes in a client-side decorated window's transparent shadow,
// the desktop behind it. Not covered: reading the properties, which
// the X11 snap test in tests/capture_start.rs drives.
#[cfg(test)]
mod tests {
    use super::shown_rect;
    use crate::capture::WinRect;

    const CLIENT: WinRect = WinRect {
        x: 100,
        y: 200,
        width: 400,
        height: 300,
    };

    fn shown(frame: [u32; 4], margin: [u32; 4]) -> (i32, i32, u32, u32) {
        let r = shown_rect(CLIENT, frame, margin);
        (r.x, r.y, r.width, r.height)
    }

    #[test]
    fn a_window_managers_frame_grows_the_rect_on_each_side() {
        assert_eq!(shown([0; 4], [0; 4]), (100, 200, 400, 300));
        // Left 1, right 2, top 30, bottom 4: every side apart.
        assert_eq!(shown([1, 2, 30, 4], [0; 4]), (99, 170, 403, 334));
    }

    #[test]
    fn a_client_side_shadow_shrinks_the_rect_on_each_side() {
        assert_eq!(shown([0; 4], [10, 20, 5, 40]), (110, 205, 370, 255));
        // Both set: the frame grows and the shadow shrinks.
        assert_eq!(shown([1, 2, 30, 4], [10, 20, 5, 40]), (109, 175, 373, 289));
    }

    #[test]
    fn a_rect_with_no_area_or_out_of_range_is_the_client() {
        let client = (100, 200, 400, 300);
        assert_eq!(shown([0; 4], [200, 200, 0, 0]), client);
        assert_eq!(shown([0; 4], [0, 0, 300, 1]), client);
        assert_eq!(shown([u32::MAX, 0, 0, 0], [0; 4]), client);
        assert_eq!(shown([0, 0, 0, u32::MAX], [0; 4]), client);
        // One pixel left: still a window.
        assert_eq!(shown([0; 4], [199, 200, 0, 0]), (299, 200, 1, 300));
    }
}
