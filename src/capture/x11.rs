use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, ImageFormat};

use super::{Frame, WinRect};

/// Active monitor rectangles in root (frame) pixels, primary first.
/// The overlay opens one window per monitor, each showing its slice
/// of the frozen frame at native scale. Falls back to the whole
/// root when the WM reports no monitors.
pub fn monitors() -> Result<Vec<WinRect>, String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
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

/// Monitors and snappable windows over one X connection: the capture
/// path queries both back to back, and a connection handshake is
/// tens of ms of dead latency before the overlay can open.
pub fn layout() -> Result<(Vec<WinRect>, Vec<WinRect>), String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let reply = conn
        .randr_get_monitors(root, true)
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
    let windows = list_top_level_windows_on(&conn, screen_num)?;
    Ok((primary, windows))
}

/// Top-level windows in bottom-to-top stacking order (EWMH
/// _NET_CLIENT_LIST_STACKING), minus this process's own windows (the
/// frozen-frame overlay itself) and iconic/hidden ones. The overlay uses
/// this for hover-snap: the last rect containing the cursor is the
/// topmost candidate.
pub fn list_top_level_windows() -> Result<Vec<WinRect>, String> {
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
    list_top_level_windows_on(&conn, screen_num)
}

fn list_top_level_windows_on(
    conn: &impl Connection,
    screen_num: usize,
) -> Result<Vec<WinRect>, String> {
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let intern = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .map_err(|e| format!("intern {name}: {e}"))?
            .reply()
            .map(|r| r.atom)
            .map_err(|e| format!("intern {name} reply: {e}"))
    };
    let stacking = intern("_NET_CLIENT_LIST_STACKING")?;
    let wm_pid = intern("_NET_WM_PID")?;
    let wm_state = intern("_NET_WM_STATE")?;
    let state_hidden = intern("_NET_WM_STATE_HIDDEN")?;

    let list = conn
        .get_property(false, root, stacking, AtomEnum::WINDOW, 0, u32::MAX)
        .map_err(|e| format!("query stacking: {e}"))?
        .reply()
        .map_err(|e| format!("query stacking reply: {e}"))?;
    let mut windows: Vec<u32> = list.value32().map(|v| v.collect()).unwrap_or_default();
    if windows.is_empty() {
        // No window manager (or none that sets EWMH lists): fall back to
        // the root's child list, which is bottom-to-top stacking order.
        let tree = conn
            .query_tree(root)
            .map_err(|e| format!("query_tree: {e}"))?
            .reply()
            .map_err(|e| format!("query_tree reply: {e}"))?;
        for win in tree.children {
            let viewable = conn
                .get_window_attributes(win)
                .ok()
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
    }

    let own_pid = std::process::id();
    let get_prop = |win: u32, atom: u32, typ: x11rb::protocol::xproto::Atom, len: u32| {
        conn.get_property(false, win, atom, typ, 0, len)
            .ok()?
            .reply()
            .ok()
    };
    let mut rects = Vec::with_capacity(windows.len());
    for win in windows {
        // Own windows (overlay, chips, borders) must never be snappable.
        if let Some(reply) = get_prop(win, wm_pid, AtomEnum::CARDINAL.into(), 1) {
            if let Some(mut it) = reply.value32() {
                if it.next() == Some(own_pid) {
                    continue;
                }
            }
        }
        // Skip minimized windows.
        if let Some(reply) = get_prop(win, wm_state, AtomEnum::ATOM.into(), 32) {
            if reply
                .value32()
                .map(|mut v| v.any(|s| s == state_hidden))
                .unwrap_or(false)
            {
                continue;
            }
        }
        let geom = match conn
            .get_geometry(win)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(g) => g,
            None => continue, // window vanished mid-enumeration
        };
        let origin = match conn
            .translate_coordinates(win, root, 0, 0)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(t) => t,
            None => continue,
        };
        if geom.width == 0 || geom.height == 0 {
            continue;
        }
        rects.push(WinRect {
            x: i32::from(origin.dst_x) - i32::from(geom.border_width),
            y: i32::from(origin.dst_y) - i32::from(geom.border_width),
            width: u32::from(geom.width) + 2 * u32::from(geom.border_width),
            height: u32::from(geom.height) + 2 * u32::from(geom.border_width),
        });
    }
    Ok(rects)
}

/// X11 full-screen capture via GetImage on the root window.
///
/// The root window spans the whole virtual screen, so one grab covers every
/// monitor; the frozen frame's coordinates match root coordinates, which is
/// what the overlay selection reports back.
pub struct X11Backend;

impl X11Backend {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}

impl super::CaptureBackend for X11Backend {
    fn grab_screen(&self) -> Result<Frame, String> {
        let (conn, screen_num) =
            x11rb::connect(None).map_err(|e| format!("X11 connect: {e}"))?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;

        let geom = conn
            .get_geometry(root)
            .map_err(|e| format!("X11 get_geometry: {e}"))?
            .reply()
            .map_err(|e| format!("X11 get_geometry reply: {e}"))?;
        let width = u32::from(geom.width);
        let height = u32::from(geom.height);

        let pixels = width as usize * height as usize;
        let mut rgba = vec![0u8; pixels * 4];
        let depth = grab_pixels_into(&conn, root, geom.width, geom.height, &mut rgba)?;

        if depth != 24 {
            return Err(format!(
                "unsupported root depth {}; only 24-bit TrueColor is implemented",
                depth
            ));
        }

        Ok(Frame { width, height, rgba })
    }
}

/// BGRX/BGR/raw-24 to opaque RGBA, banded across threads at 4K sizes.
/// `src` may be a socket reply or a mapped SHM segment; either way the
/// conversion writes `out` exactly once.
fn convert_to_rgba(
    src: &[u8],
    pixels: usize,
    bpp: usize,
    out: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), String> {
    match bpp {
        // XRGB/BGRX little-endian: B, G, R, _ per pixel.
        4 => {
            // Parallel u32 swizzle: at 12M pixels a scalar
            // per-byte loop is a visible slice of the latency.
            let threads = std::thread::available_parallelism()
                .map(|n| n.get().min(8))
                .unwrap_or(4);
            let chunk_px = pixels.div_ceil(threads);
            std::thread::scope(|scope| {
                let mut out_rest = out;
                let mut in_rest = src;
                for _ in 0..threads {
                    let take_px = chunk_px.min(in_rest.len() / 4);
                    if take_px == 0 {
                        break;
                    }
                    let (out_chunk, o_rest) = out_rest.split_at_mut(take_px * 4);
                    let (in_chunk, i_rest) = in_rest.split_at(take_px * 4);
                    out_rest = o_rest;
                    in_rest = i_rest;
                    scope.spawn(move || {
                        for (o, i) in out_chunk
                            .chunks_exact_mut(4)
                            .zip(in_chunk.chunks_exact(4))
                        {
                            let v = u32::from_le_bytes([i[0], i[1], i[2], i[3]]);
                            let rgb = (v & 0xFF00_FF00)
                                | ((v & 0xFF) << 16)
                                | ((v >> 16) & 0xFF);
                            o.copy_from_slice(&(rgb | 0xFF00_0000).to_le_bytes());
                        }
                    });
                }
            });
        }
        3 => {
            for (o, px) in out.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                o.copy_from_slice(&[px[2], px[1], px[0], 255]);
            }
        }
        other => {
            return Err(format!(
                "unsupported bytes-per-pixel {other} for {}x{} grab",
                width, height
            ));
        }
    }
    Ok(())
}

/// The pixel transfer, converted into `out`. A 4K-and-change root is
/// ~200MB: over the X socket that is seconds, over MIT-SHM it is a
/// page-faulted read. The SHM path converts straight out of the mapped
/// segment — no intermediate copy of the frame ever exists. Falls back
/// to plain GetImage when the server lacks SHM or the segment cannot
/// be set up. Returns the image depth.
fn grab_pixels_into<C>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    width: u16,
    height: u16,
    out: &mut [u8],
) -> Result<u8, String>
where
    C: Connection + x11rb::protocol::xproto::ConnectionExt,
{
    if let Some(depth) = try_shm_grab_into(conn, root, width, height, out) {
        return Ok(depth);
    }
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, width, height, !0u32)
        .map_err(|e| format!("X11 get_image: {e}"))?
        .reply()
        .map_err(|e| format!("X11 get_image reply: {e}"))?;
    if reply.depth == 24 {
        let pixels = width as usize * height as usize;
        let bpp = reply.data.len() / pixels.max(1);
        convert_to_rgba(&reply.data, pixels, bpp, out, u32::from(width), u32::from(height))?;
    }
    Ok(reply.depth)
}

fn try_shm_grab_into<C>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    width: u16,
    height: u16,
    out: &mut [u8],
) -> Option<u8>
where
    C: Connection + x11rb::protocol::xproto::ConnectionExt,
{
    use x11rb::protocol::shm::ConnectionExt as ShmExt;
    let version = ShmExt::shm_query_version(conn).ok()?.reply().ok()?;
    if version.major_version < 1 {
        return None;
    }
    let size = width as usize * height as usize * 4;
    unsafe {
        let shmid = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
        if shmid < 0 {
            return None;
        }
        // Mark for removal now: the segment dies with the last detach,
        // so no crash path can leak it.
        libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
        let addr = libc::shmat(shmid, std::ptr::null(), 0);
        if addr as isize == -1 {
            return None;
        }
        let result = (|| -> Option<u8> {
            let seg = conn.generate_id().ok()?;
            ShmExt::shm_attach(conn, seg, shmid as u32, false).ok()?.check().ok()?;
            let reply = ShmExt::shm_get_image(
                conn, root, 0, 0, width, height, !0u32, ImageFormat::Z_PIXMAP.into(), seg, 0,
            )
            .ok()?
            .reply()
            .ok()?;
            if reply.depth == 24 {
                let src = std::slice::from_raw_parts(addr as *const u8, size);
                let pixels = width as usize * height as usize;
                let bpp = src.len() / pixels.max(1);
                convert_to_rgba(src, pixels, bpp, out, u32::from(width), u32::from(height)).ok()?;
            }
            let _ = ShmExt::shm_detach(conn, seg);
            Some(reply.depth)
        })();
        libc::shmdt(addr);
        result
    }
}
