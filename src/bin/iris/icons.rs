//! Vector icons: one stroke-weighted set for every chrome glyph.
//!
//! Text labels and emoji do not ship on chrome. Every icon is drawn
//! into an 18x18 logical box (scaled to the element bounds) with the
//! same 1.6px-equivalent stroke, using the path-tessellation helpers
//! the editor's vector markup already uses. One owner for the helpers:
//! editor.rs paints through these too.

use gpui::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Pen,
    Line,
    Arrow,
    Ellipse,
    Rect,
    Text,
    Highlight,
    Blur,
    Undo,
    Redo,
    Trash,
    ChevronDown,
    Check,
    Copy,
    Close,
    Mic,
    Stroke1,
    Stroke2,
    Stroke3,
    Cursor,
    Crop,
    Viewfinder,
    Grid,
    Gear,
    Folder,
    Pin,
    Pause,
    MicOff,
    Counter,
    Record,
    Play,
}

/// A `size`px square canvas that paints `kind` in `color`.
pub fn icon(kind: Icon, color: Rgba, size: f32) -> impl IntoElement {
    canvas(
        move |_, _, _| (),
        move |bounds, (), window, _| paint_icon(kind, bounds, color, window),
    )
    .w(px(size))
    .h(px(size))
}

fn paint_icon(kind: Icon, bounds: Bounds<Pixels>, color: Rgba, window: &mut Window) {
    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
    let s: f32 = f32::from(bounds.size.width) / 18.0;
    let p = move |x: f32, y: f32| point(px(ox + x * s), px(oy + y * s));
    let w = 1.6 * s;

    let mut path = Path::new(p(0.0, 0.0));
    match kind {
        Icon::Pen => {
            // Barrel from upper right toward the nib, then the nib tip.
            push_segment(&mut path, p(6.8, 11.2), p(13.2, 4.8), 2.8 * s);
            push_filled_triangle(&mut path, p(3.4, 14.6), p(4.4, 10.8), p(7.2, 13.6));
        }
        Icon::Line => {
            push_segment(&mut path, p(4.0, 14.0), p(14.0, 4.0), w);
            push_disc(&mut path, p(4.0, 14.0), 2.2 * s);
            push_disc(&mut path, p(14.0, 4.0), 2.2 * s);
        }
        Icon::Arrow => {
            let (a, b) = (p(4.0, 14.0), p(13.2, 4.8));
            push_segment(&mut path, a, b, w);
            let head = 6.0 * s;
            let angle = (f32::from(b.y) - f32::from(a.y)).atan2(f32::from(b.x) - f32::from(a.x));
            let h1 = point(
                b.x - px(head * (angle - 0.5).cos()),
                b.y - px(head * (angle - 0.5).sin()),
            );
            let h2 = point(
                b.x - px(head * (angle + 0.5).cos()),
                b.y - px(head * (angle + 0.5).sin()),
            );
            push_filled_triangle(&mut path, b, h1, h2);
        }
        Icon::Ellipse => {
            push_ring(&mut path, p(9.0, 9.0), 5.6 * s, 4.2 * s, w);
        }
        Icon::Viewfinder => {
            // Four corner brackets: the region-select reticle.
            let w2 = 1.8 * s;
            push_segment(&mut path, p(3.2, 6.6), p(3.2, 3.2), w2);
            push_segment(&mut path, p(3.2, 3.2), p(6.6, 3.2), w2);
            push_segment(&mut path, p(11.4, 3.2), p(14.8, 3.2), w2);
            push_segment(&mut path, p(14.8, 3.2), p(14.8, 6.6), w2);
            push_segment(&mut path, p(3.2, 11.4), p(3.2, 14.8), w2);
            push_segment(&mut path, p(3.2, 14.8), p(6.6, 14.8), w2);
            push_segment(&mut path, p(11.4, 14.8), p(14.8, 14.8), w2);
            push_segment(&mut path, p(14.8, 14.8), p(14.8, 11.4), w2);
        }
        Icon::Grid => {
            // 2x2 rounded squares: a library of captures.
            for (gx, gy) in [(4.0f32, 4.0f32), (10.0, 4.0), (4.0, 10.0), (10.0, 10.0)] {
                let (a, b) = (p(gx, gy), p(gx + 4.0, gy + 4.0));
                push_segment(&mut path, a, point(b.x, a.y), w);
                push_segment(&mut path, point(b.x, a.y), b, w);
                push_segment(&mut path, b, point(a.x, b.y), w);
                push_segment(&mut path, point(a.x, b.y), a, w);
            }
        }
        Icon::Gear => {
            // Ring with eight spokes.
            push_ring(&mut path, p(9.0, 9.0), 3.4 * s, 3.4 * s, w);
            let c = p(9.0, 9.0);
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                let (r0, r1) = (4.8 * s, 6.6 * s);
                push_segment(
                    &mut path,
                    point(c.x + px(r0 * a.cos()), c.y + px(r0 * a.sin())),
                    point(c.x + px(r1 * a.cos()), c.y + px(r1 * a.sin())),
                    1.5 * s,
                );
            }
        }
        Icon::Rect => {
            let (a, b) = (p(4.2, 5.4), p(13.8, 12.6));
            push_segment(&mut path, a, point(b.x, a.y), w);
            push_segment(&mut path, point(b.x, a.y), b, w);
            push_segment(&mut path, b, point(a.x, b.y), w);
            push_segment(&mut path, point(a.x, b.y), a, w);
        }
        Icon::Text => {
            push_segment(&mut path, p(4.4, 5.2), p(13.6, 5.2), w);
            push_segment(&mut path, p(9.0, 5.2), p(9.0, 14.2), w);
        }
        Icon::Highlight => {
            // Marker stroke plus the line it sits on.
            push_segment(&mut path, p(4.6, 10.4), p(11.6, 3.4), 3.4 * s);
            push_segment(&mut path, p(3.0, 14.6), p(15.0, 14.6), w);
        }
        Icon::Blur => {
            // Pixelated patch: outline plus an inner mosaic.
            let (a, b) = (p(4.2, 4.2), p(13.8, 13.8));
            push_segment(&mut path, a, point(b.x, a.y), w);
            push_segment(&mut path, point(b.x, a.y), b, w);
            push_segment(&mut path, b, point(a.x, b.y), w);
            push_segment(&mut path, point(a.x, b.y), a, w);
            for (qx, qy) in [(6.4, 6.4), (9.6, 6.4), (6.4, 9.6), (9.6, 9.6)] {
                push_quad(
                    &mut path,
                    p(qx, qy),
                    p(qx + 2.0, qy),
                    p(qx + 2.0, qy + 2.0),
                    p(qx, qy + 2.0),
                );
            }
        }
        Icon::Undo => paint_undo(&mut path, p, s, w, false),
        Icon::Redo => paint_undo(&mut path, p, s, w, true),
        Icon::Trash => {
            push_segment(&mut path, p(4.4, 5.6), p(13.6, 5.6), w);
            push_segment(&mut path, p(7.4, 4.0), p(10.6, 4.0), w);
            push_segment(&mut path, p(5.4, 5.6), p(6.2, 14.2), w);
            push_segment(&mut path, p(12.6, 5.6), p(11.8, 14.2), w);
            push_segment(&mut path, p(6.2, 14.2), p(11.8, 14.2), w);
            push_segment(&mut path, p(8.0, 7.6), p(8.3, 12.2), w * 0.85);
            push_segment(&mut path, p(10.0, 7.6), p(9.7, 12.2), w * 0.85);
        }
        Icon::ChevronDown => {
            push_segment(&mut path, p(5.4, 7.0), p(9.0, 10.6), w);
            push_segment(&mut path, p(9.0, 10.6), p(12.6, 7.0), w);
        }
        Icon::Check => {
            push_segment(&mut path, p(4.4, 9.6), p(7.8, 13.0), w);
            push_segment(&mut path, p(7.8, 13.0), p(13.6, 5.2), w);
        }
        Icon::Copy => {
            // Back sheet upper right, front sheet lower left.
            let (ba, bb) = (p(7.0, 4.0), p(14.0, 11.0));
            push_segment(&mut path, ba, point(bb.x, ba.y), w);
            push_segment(&mut path, point(bb.x, ba.y), bb, w);
            push_segment(&mut path, bb, point(ba.x, bb.y), w);
            push_segment(&mut path, point(ba.x, bb.y), ba, w);
            let (fa, fb) = (p(4.0, 7.0), p(11.0, 14.0));
            push_segment(&mut path, fa, point(fb.x, fa.y), w);
            push_segment(&mut path, point(fb.x, fa.y), fb, w);
            push_segment(&mut path, fb, point(fa.x, fb.y), w);
            push_segment(&mut path, point(fa.x, fb.y), fa, w);
        }
        Icon::Close => {
            push_segment(&mut path, p(5.2, 5.2), p(12.8, 12.8), w);
            push_segment(&mut path, p(12.8, 5.2), p(5.2, 12.8), w);
        }
        Icon::Stroke1 => push_segment(&mut path, p(4.0, 9.0), p(14.0, 9.0), 1.4 * s),
        Icon::Stroke2 => push_segment(&mut path, p(4.0, 9.0), p(14.0, 9.0), 2.6 * s),
        Icon::Stroke3 => push_segment(&mut path, p(4.0, 9.0), p(14.0, 9.0), 4.2 * s),
        Icon::Cursor => {
            // Pointer arrow as a filled fan of triangles.
            let tip = p(5.0, 3.0);
            let pts = [
                p(5.0, 15.0),
                p(8.4, 11.6),
                p(10.6, 15.8),
                p(12.4, 14.8),
                p(10.2, 10.6),
                p(14.2, 10.2),
            ];
            for tri in pts.windows(2) {
                push_filled_triangle(&mut path, tip, tri[0], tri[1]);
            }
        }
        Icon::Crop => {
            // Four corner brackets.
            for (cx, cy, dx, dy) in [
                (4.0, 4.0, 1.0, 1.0),
                (14.0, 4.0, -1.0, 1.0),
                (4.0, 14.0, 1.0, -1.0),
                (14.0, 14.0, -1.0, -1.0),
            ] {
                push_segment(&mut path, p(cx, cy + 4.0 * dy), p(cx, cy), w);
                push_segment(&mut path, p(cx, cy), p(cx + 4.0 * dx, cy), w);
            }
        }
        Icon::Mic => {
            // Capsule, pickup arc, stem, foot.
            push_ring(&mut path, p(9.0, 6.6), 2.6 * s, 3.8 * s, w);
            let (cx, cy, r) = (9.0, 8.6, 4.6);
            let n = 12;
            let (a0, a1) = (0.15f32, std::f32::consts::PI - 0.15);
            let pt = |t: f32| p(cx + r * t.cos(), cy + r * t.sin());
            for i in 0..n {
                let t0 = a0 + (a1 - a0) * i as f32 / n as f32;
                let t1 = a0 + (a1 - a0) * (i + 1) as f32 / n as f32;
                push_segment(&mut path, pt(t0), pt(t1), w);
            }
            push_segment(&mut path, p(9.0, 13.2), p(9.0, 15.4), w);
            push_segment(&mut path, p(6.4, 15.4), p(11.6, 15.4), w);
        }
        Icon::Folder => {
            // Folder outline: top tab, body below.
            push_segment(&mut path, p(3.2, 5.2), p(7.2, 5.2), w);
            push_segment(&mut path, p(7.2, 5.2), p(8.8, 7.2), w);
            push_segment(&mut path, p(8.8, 7.2), p(14.8, 7.2), w);
            push_segment(&mut path, p(14.8, 7.2), p(14.8, 14.0), w);
            push_segment(&mut path, p(14.8, 14.0), p(3.2, 14.0), w);
            push_segment(&mut path, p(3.2, 14.0), p(3.2, 5.2), w);
        }
        Icon::Pin => {
            // Diagonal pushpin: head top-right, needle pointing bottom-left.
            push_segment(&mut path, p(8.5, 9.5), p(3.5, 14.5), w);
            push_segment(&mut path, p(6.0, 7.0), p(11.0, 12.0), 1.8 * s);
            push_segment(&mut path, p(8.5, 9.5), p(13.0, 5.0), 3.0 * s);
            push_segment(&mut path, p(11.5, 2.5), p(15.5, 6.5), w);
        }
        Icon::Pause => {
            // Two bars.
            push_segment(&mut path, p(6.6, 4.5), p(6.6, 13.5), 2.4 * s);
            push_segment(&mut path, p(11.4, 4.5), p(11.4, 13.5), 2.4 * s);
        }
        Icon::MicOff => {
            // Mic capsule plus a slash through it.
            push_ring(&mut path, p(9.0, 6.6), 2.6 * s, 3.8 * s, w);
            push_segment(&mut path, p(4.0, 4.0), p(14.0, 14.0), w);
            push_segment(&mut path, p(9.0, 13.2), p(9.0, 15.4), w);
        }
        Icon::Counter => {
            // Numbered badge: filled disc with a "1" stem.
            push_disc(&mut path, p(9.0, 9.0), 6.4 * s);
            push_segment(&mut path, p(9.0, 5.6), p(9.0, 12.4), 1.8 * s);
            push_segment(&mut path, p(7.4, 7.2), p(9.0, 5.6), 1.8 * s);
        }
        Icon::Record => {
            // Video camera: body rect, lens triangle on the right.
            push_segment(&mut path, p(3.0, 5.5), p(11.0, 5.5), w);
            push_segment(&mut path, p(11.0, 5.5), p(11.0, 12.5), w);
            push_segment(&mut path, p(11.0, 12.5), p(3.0, 12.5), w);
            push_segment(&mut path, p(3.0, 12.5), p(3.0, 5.5), w);
            push_segment(&mut path, p(11.0, 7.5), p(15.0, 5.0), w);
            push_segment(&mut path, p(15.0, 5.0), p(15.0, 13.0), w);
            push_segment(&mut path, p(15.0, 13.0), p(11.0, 10.5), w);
        }
        Icon::Play => {
            // Right-pointing triangle.
            push_filled_triangle(&mut path, p(6.0, 4.0), p(6.0, 14.0), p(14.5, 9.0));
        }
    }
    window.paint_path(path, color);
}

/// Curved arrow arc with the head at its start; mirrored for redo.
fn paint_undo(
    path: &mut Path<Pixels>,
    p: impl Fn(f32, f32) -> Point<Pixels>,
    s: f32,
    w: f32,
    mirror: bool,
) {
    let (cx, cy, r) = (9.2, 10.2, 4.4);
    let (a0, a1) = if mirror {
        (std::f32::consts::PI + 0.9, -0.26f32)
    } else {
        (-0.9f32, 3.4f32)
    };
    let n = 18;
    let pt = |t: f32| p(cx + r * t.cos(), cy + r * t.sin());
    for i in 0..n {
        let t0 = a0 + (a1 - a0) * i as f32 / n as f32;
        let t1 = a0 + (a1 - a0) * (i + 1) as f32 / n as f32;
        push_segment(path, pt(t0), pt(t1), w);
    }
    let tip = pt(a0);
    let back = pt(if mirror { a0 + 0.28 } else { a0 - 0.28 });
    let (tx, ty): (f32, f32) = (tip.x.into(), tip.y.into());
    let (bx, by): (f32, f32) = (back.x.into(), back.y.into());
    let len = ((bx - tx).powi(2) + (by - ty).powi(2)).sqrt().max(0.001);
    let (dx, dy) = ((bx - tx) / len, (by - ty) / len);
    let (nx, ny) = (-dy, dx);
    let (hl, hw) = (5.4 * s, 3.0 * s);
    push_filled_triangle(
        path,
        point(px(tx + dx * 1.2 * s), px(ty + dy * 1.2 * s)),
        point(px(tx - dx * hl + nx * hw), px(ty - dy * hl + ny * hw)),
        point(px(tx - dx * hl - nx * hw), px(ty - dy * hl - ny * hw)),
    );
}

/// A thick segment as a filled rectangle plus round caps.
pub(crate) fn push_segment(path: &mut Path<Pixels>, a: Point<Pixels>, b: Point<Pixels>, w: f32) {
    let (ax, ay): (f32, f32) = (a.x.into(), a.y.into());
    let (bx, by): (f32, f32) = (b.x.into(), b.y.into());
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
    if len < 0.001 {
        push_disc(path, a, w / 2.0);
        return;
    }
    let (nx, ny) = (-(by - ay) / len * w / 2.0, (bx - ax) / len * w / 2.0);
    push_quad(
        path,
        point(a.x + px(nx), a.y + px(ny)),
        point(b.x + px(nx), b.y + px(ny)),
        point(b.x - px(nx), b.y - px(ny)),
        point(a.x - px(nx), a.y - px(ny)),
    );
    push_disc(path, a, w / 2.0);
    push_disc(path, b, w / 2.0);
}

pub(crate) fn push_disc(path: &mut Path<Pixels>, c: Point<Pixels>, r: f32) {
    let (cx, cy): (f32, f32) = (c.x.into(), c.y.into());
    let n = 20;
    for i in 0..n {
        let t0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let t1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        push_filled_triangle(
            path,
            c,
            point(px(cx + r * t0.cos()), px(cy + r * t0.sin())),
            point(px(cx + r * t1.cos()), px(cy + r * t1.sin())),
        );
    }
}

/// An elliptical ring outline, `w` thick.
pub(crate) fn push_ring(path: &mut Path<Pixels>, c: Point<Pixels>, rx: f32, ry: f32, w: f32) {
    let (cx, cy): (f32, f32) = (c.x.into(), c.y.into());
    let n = ((rx + ry) * 0.35).max(32.0) as usize;
    for i in 0..n {
        let t0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let t1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let (ro_x, ro_y) = (rx + w / 2.0, ry + w / 2.0);
        let (ri_x, ri_y) = ((rx - w / 2.0).max(0.1), (ry - w / 2.0).max(0.1));
        let o0 = point(px(cx + ro_x * t0.cos()), px(cy + ro_y * t0.sin()));
        let o1 = point(px(cx + ro_x * t1.cos()), px(cy + ro_y * t1.sin()));
        let i0 = point(px(cx + ri_x * t0.cos()), px(cy + ri_y * t0.sin()));
        let i1 = point(px(cx + ri_x * t1.cos()), px(cy + ri_y * t1.sin()));
        push_quad(path, o0, o1, i1, i0);
    }
}

/// A filled ellipse (interior, no ring).
pub(crate) fn push_ellipse_fill(path: &mut Path<Pixels>, c: Point<Pixels>, rx: f32, ry: f32) {
    let (cx, cy): (f32, f32) = (c.x.into(), c.y.into());
    let n = ((rx + ry) * 0.35).max(32.0) as usize;
    for i in 0..n {
        let t0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let t1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        push_filled_triangle(
            path,
            c,
            point(px(cx + rx * t0.cos()), px(cy + ry * t0.sin())),
            point(px(cx + rx * t1.cos()), px(cy + ry * t1.sin())),
        );
    }
}

/// A filled axis-aligned rectangle.
pub(crate) fn push_rect_fill(path: &mut Path<Pixels>, a: Point<Pixels>, b: Point<Pixels>) {
    push_quad(path, a, point(b.x, a.y), b, point(a.x, b.y));
}

pub(crate) fn push_quad(
    path: &mut Path<Pixels>,
    a: Point<Pixels>,
    b: Point<Pixels>,
    c: Point<Pixels>,
    d: Point<Pixels>,
) {
    push_filled_triangle(path, a, b, c);
    push_filled_triangle(path, a, c, d);
}

pub(crate) fn push_filled_triangle(
    path: &mut Path<Pixels>,
    a: Point<Pixels>,
    b: Point<Pixels>,
    c: Point<Pixels>,
) {
    path.push_triangle(
        (a, b, c),
        (point(0., 1.), point(0., 1.), point(0., 1.)),
    );
}
