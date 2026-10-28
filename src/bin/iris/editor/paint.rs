//! GPU path tessellation and painting for vector actions.

use std::rc::Rc;

use gpui::*;

use super::action::{hex_rgba, Action, Tool};
use super::raster_shapes::arrow_wings;
use crate::icons::{
    self, push_disc, push_ellipse_fill, push_filled_triangle, push_rect_fill, push_ring,
    push_segment, TriSink,
};
use crate::theme;

/// Tessellate and paint one action on the stage canvas. Coordinates are
/// image pixels scaled into stage space.
pub(crate) fn paint_action(
    action: &Action,
    bounds: Bounds<Pixels>,
    scale: f32,
    _base_w: u32,
    _base_h: u32,
    window: &mut Window,
) {
    if action.tool == Tool::Blur || action.tool == Tool::Text || action.points.is_empty() {
        return;
    }
    let c = hex_rgba(action.color);
    let color = if action.tool == Tool::Highlight {
        theme::alpha(c, 0.35)
    } else {
        c
    };
    let w = action.width * scale;
    let (ox, oy): (f32, f32) = (bounds.origin.x.into(), bounds.origin.y.into());
    let s = move |p: (f32, f32)| point(px(p.0 * scale), px(p.1 * scale));
    // Cache hit: the triangles for these exact points at this scale
    // are already tessellated; re-stamping them at this paint's
    // origin is one add per vertex instead of re-tessellating every
    // segment's quad and cap discs. A pan drag changes only the
    // origin, so it hits this path every frame.
    let fingerprint = (
        scale,
        action.points.len(),
        *action.points.first().unwrap(),
        *action.points.last().unwrap(),
    );
    {
        let mut cache = action.cached_path.borrow_mut();
        if let Some((kscale, klen, kfirst, klast, cached)) = cache.as_mut() {
            if (*kscale, *klen, *kfirst, *klast) == fingerprint {
                let mut path = Path::new(point(px(ox), px(oy)));
                stamp_tris(&mut path, cached, ox, oy);
                paint_stroke(path, color, window);
                return;
            }
            // Incremental: a growing pen/highlight stroke keeps its scale
            // and first point, so the new segments append onto the cached
            // triangles instead of re-tessellating the whole polyline on
            // every mousemove (O(stroke) per move became O(new segments)).
            // The cache holds the only Rc, so make_mut appends in place:
            // cloning the Vec per move was still an O(stroke) copy.
            if matches!(action.tool, Tool::Pen | Tool::Highlight)
                && (*kscale, *kfirst) == (scale, fingerprint.2)
                && *klen >= 2
                && *klen < action.points.len()
            {
                let tris = Rc::make_mut(cached);
                let mut rec = icons::TriRecorder(std::mem::take(tris));
                for seg in action.points[*klen - 1..].windows(2) {
                    push_segment(&mut rec, s(seg[0]), s(seg[1]), w);
                }
                *tris = rec.0;
                *klen = action.points.len();
                *klast = fingerprint.3;
                let mut path = Path::new(point(px(ox), px(oy)));
                stamp_tris(&mut path, tris, ox, oy);
                paint_stroke(path, color, window);
                return;
            }
        }
    }

    // Cache miss: tessellate at origin (0,0) into the recorder, then
    // stamp at this paint's origin.
    let mut rec = icons::TriRecorder(Vec::new());
    tessellate(action, scale, &mut rec);
    let tris = Rc::new(rec.0);
    let mut path = Path::new(point(px(ox), px(oy)));
    stamp_tris(&mut path, &tris, ox, oy);
    *action.cached_path.borrow_mut() = Some((
        fingerprint.0,
        fingerprint.1,
        fingerprint.2,
        fingerprint.3,
        tris,
    ));
    paint_stroke(path, color, window);
}

/// Tessellate `action` into `path` at stage origin (0, 0), with image
/// pixels scaled by `scale`. Every size is in image pixels, as in the
/// saved file's raster, so the canvas shows the shape the file gets at
/// any zoom.
pub(crate) fn tessellate(action: &Action, scale: f32, path: &mut impl TriSink) {
    let w = action.width * scale;
    let s = |p: (f32, f32)| point(px(p.0 * scale), px(p.1 * scale));
    let ends = || Some((s(*action.points.first()?), s(*action.points.last()?)));
    match action.tool {
        Tool::Pen | Tool::Highlight => {
            if let [p] = action.points[..] {
                push_disc(path, s(p), w / 2.0);
            } else {
                for seg in action.points.windows(2) {
                    push_segment(path, s(seg[0]), s(seg[1]), w);
                }
            }
        }
        Tool::Line => {
            if let Some((a, b)) = ends() {
                push_segment(path, a, b, w);
            }
        }
        Tool::Arrow => {
            if let (Some(&a), Some(&b)) = (action.points.first(), action.points.last()) {
                push_segment(path, s(a), s(b), w);
                let [p1, p2] = arrow_wings(a, b, action.width);
                push_filled_triangle(path, s(b), s(p1), s(p2));
            }
        }
        Tool::Ellipse => {
            if let Some((a, b)) = ends() {
                let cx = (f32::from(a.x) + f32::from(b.x)) / 2.0;
                let cy = (f32::from(a.y) + f32::from(b.y)) / 2.0;
                let rx = (f32::from(b.x) - f32::from(a.x)).abs() / 2.0;
                let ry = (f32::from(b.y) - f32::from(a.y)).abs() / 2.0;
                if rx > 0.0 && ry > 0.0 {
                    if action.filled {
                        push_ellipse_fill(path, point(px(cx), px(cy)), rx, ry);
                    } else {
                        push_ring(path, point(px(cx), px(cy)), rx, ry, w);
                    }
                }
            }
        }
        Tool::Rect => {
            if let Some((a, b)) = ends() {
                if action.filled {
                    push_rect_fill(path, a, b);
                } else {
                    push_segment(path, a, point(b.x, a.y), w);
                    push_segment(path, point(b.x, a.y), b, w);
                    push_segment(path, b, point(a.x, b.y), w);
                    push_segment(path, point(a.x, b.y), a, w);
                }
            }
        }
        Tool::Counter => {
            if let Some(p) = action.points.first() {
                push_disc(path, s(*p), action.font_size * 0.9 * scale);
            }
        }
        Tool::Select | Tool::Text | Tool::Blur | Tool::Crop => {}
    }
}

/// Paint `path` in `color`. The path rasterizer blends each triangle over
/// the last, and a stroke's path is a quad and two round caps per
/// segment, which overlap at every joint and all along a densely sampled
/// stroke. In an opaque color the overlaps do not show. In a translucent
/// one (the highlight) they add up toward the opaque color, so the path
/// is drawn white as a clip instead and the color fills it once, the one
/// blend the saved file makes.
fn paint_stroke(path: Path<Pixels>, color: Rgba, window: &mut Window) {
    if color.a >= 1.0 {
        window.paint_path(path, color);
        return;
    }
    let bounds = path.bounds;
    window.with_clip_path(path, |window| window.paint_quad(fill(bounds, color)));
}

/// Re-stamp recorded triangles at paint offset (ox, oy): the cached
/// geometry is origin-relative, so a moved stage reuses it verbatim.
pub(crate) fn stamp_tris(path: &mut Path<Pixels>, tris: &[[f32; 6]], ox: f32, oy: f32) {
    for t in tris {
        path.push_triangle(
            (
                point(px(t[0] + ox), px(t[1] + oy)),
                point(px(t[2] + ox), px(t[3] + oy)),
                point(px(t[4] + ox), px(t[5] + oy)),
            ),
            (point(0., 1.), point(0., 1.), point(0., 1.)),
        );
    }
}
