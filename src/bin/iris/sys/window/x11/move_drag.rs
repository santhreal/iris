use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

use super::{find_xid_by_class_on, shared_conn};

/// Drag the window under the press until the button is released.
/// Client-side decorations give the WM no title bar to grab, and
/// _NET_WM_MOVERESIZE loses the pointer grab against GPUI's
/// click-handling grab, so the title bar drags directly: poll the
/// root pointer and slide the window by the delta. WM-independent.
/// X11 only; `sys::window::begin_wm_move` routes Wayland elsewhere.
pub fn begin_wm_move(class: String, root_x: i32, root_y: i32) {
    std::thread::spawn(move || {
        let Some((conn, screen_num)) = shared_conn() else {
            return;
        };
        let root = conn.setup().roots[screen_num].root;
        let mut xid = None;
        for _ in 0..20 {
            if let Some(found) = find_xid_by_class_on(&conn, &class) {
                xid = Some(found);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let Some(xid) = xid else {
            return;
        };
        // The window's current root position.
        let Ok(trans) = conn.translate_coordinates(xid, root, 0, 0) else {
            return;
        };
        let Ok(trans) = trans.reply() else {
            return;
        };
        let win0 = (i32::from(trans.dst_x), i32::from(trans.dst_y));
        let anchor = (root_x, root_y);
        // Event-driven tracking: select button-1 motion and
        // release on the root so the X server pushes pointer
        // moves to this connection. The old loop polled
        // query_pointer every 8ms, a round trip each (~125/sec
        // for the life of a drag).
        let mask = x11rb::protocol::xproto::EventMask::BUTTON1_MOTION
            | x11rb::protocol::xproto::EventMask::BUTTON_RELEASE;
        let aux = x11rb::protocol::xproto::ChangeWindowAttributesAux::new().event_mask(mask);
        if conn.change_window_attributes(root, &aux).is_err() {
            return;
        }
        let _ = conn.flush();
        // The connection is process-shared: the mask must go back
        // to zero on every exit path or root motion events queue
        // on it forever.
        struct MaskReset<'a>(&'a x11rb::rust_connection::RustConnection, u32);
        impl Drop for MaskReset<'_> {
            fn drop(&mut self) {
                let aux = x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
                    .event_mask(x11rb::protocol::xproto::EventMask::default());
                let _ = self.0.change_window_attributes(self.1, &aux);
                let _ = self.0.flush();
            }
        }
        let _reset = MaskReset(conn, root);
        use std::os::unix::io::AsRawFd;
        let x_fd = conn.stream().as_raw_fd();
        loop {
            let mut released = false;
            loop {
                match conn.poll_for_event() {
                    Ok(Some(x11rb::protocol::Event::MotionNotify(ev))) => {
                        let dx = i32::from(ev.root_x) - anchor.0;
                        let dy = i32::from(ev.root_y) - anchor.1;
                        if dx != 0 || dy != 0 {
                            let aux = x11rb::protocol::xproto::ConfigureWindowAux::new()
                                .x(win0.0 + dx)
                                .y(win0.1 + dy);
                            let _ = conn.configure_window(xid, &aux);
                        }
                    }
                    Ok(Some(x11rb::protocol::Event::ButtonRelease(ev))) if ev.detail == 1 => {
                        released = true;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => return,
                }
            }
            let _ = conn.flush();
            if released {
                return;
            }
            // Sleep on the fd; the 100ms timeout is the fallback
            // for a release swallowed by another client's grab,
            // checked with one query_pointer.
            let mut pfd = libc::pollfd {
                fd: x_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            if unsafe { libc::poll(&mut pfd, 1, 100) } == 0 {
                let Ok(pointer) = conn.query_pointer(root) else {
                    return;
                };
                let Ok(pointer) = pointer.reply() else {
                    return;
                };
                if pointer.mask & x11rb::protocol::xproto::KeyButMask::BUTTON1
                    != x11rb::protocol::xproto::KeyButMask::BUTTON1
                {
                    return;
                }
            }
        }
    });
}
