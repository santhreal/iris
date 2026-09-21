use std::borrow::Cow;

use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as ShapeExt, SK, SO};
use x11rb::protocol::xproto::{
    Arc, ChangeGCAux, ConnectionExt as XprotoExt, CreateGCAux, CreateWindowAux, Gcontext,
    ImageFormat, ImageOrder, Pixmap, Rectangle, Window, WindowClass,
};

use super::DragIcon;

/// The pointer-following icon window: opaque 24-bit with the thumbnail
/// as its background pixmap and a 1-bit rounded-rect shape mask.
pub(super) struct IconWindow {
    pub(super) win: Window,
    width: i16,
    height: i16,
}

impl IconWindow {
    pub(super) fn show<C: Connection>(conn: &C, root: Window, icon: DragIcon) -> Option<Self> {
        const MAX_W: u32 = 96;
        let (mut w, mut h) = (icon.width, icon.height);
        // The resize output owns its buffer; the unresized path keeps
        // borrowing the Arc. Cow so the borrow and the owned resize
        // share one binding.
        let data: Cow<[u8]> = if w > MAX_W {
            let nh = (h as u64 * MAX_W as u64 / w as u64).max(1) as u32;
            let img: image::ImageBuffer<image::Rgba<u8>, _> =
                image::ImageBuffer::from_raw(w, h, &icon.rgba[..])?;
            w = MAX_W;
            h = nh;
            Cow::Owned(
                image::imageops::resize(&img, MAX_W, nh, image::imageops::FilterType::Triangle)
                    .into_raw(),
            )
        } else {
            Cow::Borrowed(&icon.rgba[..])
        };
        let data: &[u8] = &data;
        let depth = conn.setup().roots[0].root_depth;
        let lsb = conn.setup().image_byte_order == ImageOrder::LSB_FIRST;
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
        let c_gc = conn.create_gc(gc, root, &CreateGCAux::new()).ok()?;
        let c_put = conn
            .put_image(
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
        let c_mask = conn.create_pixmap(1, mask, root, w as u16, h as u16).ok()?;
        let c_mgc = conn
            .create_gc(mgc, mask, &CreateGCAux::new().foreground(0))
            .ok()?;
        let c_clear = conn
            .poly_fill_rectangle(
                mask,
                mgc,
                &[Rectangle {
                    x: 0,
                    y: 0,
                    width: w as u16,
                    height: h as u16,
                }],
            )
            .ok()?;
        let c_fg = conn
            .change_gc(mgc, &ChangeGCAux::new().foreground(1))
            .ok()?;
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
        let c_win = conn
            .create_window(
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
            c_pixmap, c_gc, c_put, c_mask, c_mgc, c_clear, c_fg, c_rects, c_arcs, c_win, c_shape,
            c_map, c_fgc, c_fmgc, c_fmask, c_fpix,
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

    pub(super) fn follow<C: Connection>(&self, conn: &C, px_x: i32, px_y: i32) {
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

    pub(super) fn destroy<C: Connection>(&self, conn: &C) {
        let _ = conn.destroy_window(self.win);
        let _ = conn.flush();
    }
}
