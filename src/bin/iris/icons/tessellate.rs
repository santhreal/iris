//! Path-tessellation helpers for the editor's vector markup.

use super::*;

/// Anything that accepts filled triangles: a `Path` being built for
/// paint, or a recorder caching geometry for re-stamping.
pub(crate) trait TriSink {
    fn tri(&mut self, a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>);
}

impl TriSink for Path<Pixels> {
    fn tri(&mut self, a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>) {
        self.push_triangle((a, b, c), (point(0., 1.), point(0., 1.), point(0., 1.)));
    }
}

impl<T: TriSink + ?Sized> TriSink for &mut T {
    fn tri(&mut self, a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>) {
        (**self).tri(a, b, c);
    }
}

/// Records triangles as flat xy pairs: geometry is tessellated once
/// at origin (0,0), then re-stamped at each paint's offset.
pub(crate) struct TriRecorder(pub Vec<[f32; 6]>);

impl TriSink for TriRecorder {
    fn tri(&mut self, a: Point<Pixels>, b: Point<Pixels>, c: Point<Pixels>) {
        self.0.push([
            a.x.into(),
            a.y.into(),
            b.x.into(),
            b.y.into(),
            c.x.into(),
            c.y.into(),
        ]);
    }
}

/// A thick segment as a filled rectangle plus round caps.
pub(crate) fn push_segment(path: &mut impl TriSink, a: Point<Pixels>, b: Point<Pixels>, w: f32) {
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

pub(crate) fn push_disc(path: &mut impl TriSink, c: Point<Pixels>, r: f32) {
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
pub(crate) fn push_ring(path: &mut impl TriSink, c: Point<Pixels>, rx: f32, ry: f32, w: f32) {
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
pub(crate) fn push_ellipse_fill(path: &mut impl TriSink, c: Point<Pixels>, rx: f32, ry: f32) {
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
pub(crate) fn push_rect_fill(path: &mut impl TriSink, a: Point<Pixels>, b: Point<Pixels>) {
    push_quad(path, a, point(b.x, a.y), b, point(a.x, b.y));
}

pub(crate) fn push_quad(
    path: &mut impl TriSink,
    a: Point<Pixels>,
    b: Point<Pixels>,
    c: Point<Pixels>,
    d: Point<Pixels>,
) {
    push_filled_triangle(path, a, b, c);
    push_filled_triangle(path, a, c, d);
}

pub(crate) fn push_filled_triangle(
    path: &mut impl TriSink,
    a: Point<Pixels>,
    b: Point<Pixels>,
    c: Point<Pixels>,
) {
    path.tri(a, b, c);
}
