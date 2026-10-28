//! Where each tool's ink lands: in the saved file, on the canvas, and in
//! the footprint a partial rebuild restores.
//!
//! WHY: the class closed here is "the three views of a shape disagree".
//! The saved file rasterizes an action in image pixels, the canvas
//! tessellates it at the stage's zoom, and an undo or a move restores
//! only part of the image from the base. The canvas sized the arrowhead
//! in stage pixels, so away from 1:1 it drew a head of another size than
//! the file got. The bbox left the arrowhead out, and an undo or a move
//! restored the bbox, not the footprint that pads text and counters, so
//! an undone or moved arrow or counter left ink in the saved image. The
//! cases walk every tool in `TOOLS` over shapes in several directions,
//! widths, and zooms, so a new tool or shape is checked once it is in
//! the table.
//!
//! Not covered: text and counter labels, which the canvas draws with
//! the platform's fonts; the blur, which the canvas shows as the patch
//! the raster computes.

use std::rc::Rc;

use iris_lib::history::Edit;

use super::action::{Action, Tool, TOOLS};
use super::paint::tessellate;
use super::raster::rasterize;
use super::Editor;
use crate::icons::TriRecorder;

/// The image side: room for every case's shape and its arrowhead.
const SIDE: u32 = 240;

/// How far the file's ink may reach past the exact shape, in image
/// pixels: the raster paints every pixel a row's span touches, so an
/// edge's pixel can lie a pixel past it, and that pixel's far edge
/// bounds the ink.
const SLOP: f32 = 2.0;

/// Each case's points: one point, the four directions, a reversed
/// diagonal, and a polyline with sharp turns.
fn shapes() -> Vec<Vec<(f32, f32)>> {
    vec![
        vec![(120.0, 120.0)],
        vec![(50.0, 120.0), (190.0, 120.0)],
        vec![(190.0, 120.0), (50.0, 120.0)],
        vec![(120.0, 50.0), (120.0, 190.0)],
        vec![(60.0, 70.0), (180.0, 170.0)],
        vec![(180.0, 150.0), (60.0, 90.0)],
        vec![
            (50.0, 120.0),
            (80.0, 70.0),
            (110.0, 160.0),
            (150.0, 80.0),
            (190.0, 120.0),
        ],
    ]
}

/// Every action the cases draw: each tool over each shape, at a thin, a
/// default, and a heavy width, outlined and filled.
fn actions() -> Vec<Action> {
    let mut all = Vec::new();
    for &(tool, ..) in TOOLS.iter() {
        for points in shapes() {
            for width in [1.2, 4.0, 16.0] {
                for filled in [false, true] {
                    let mut action = Action {
                        tool,
                        color: "#ff453a",
                        width,
                        points: Rc::new(points.clone()),
                        text: (tool == Tool::Text).then(|| "Ag".into()),
                        font_size: 16.0,
                        filled,
                        blur_patch: None,
                        blur_rect: (0.0, 0.0, 0.0, 0.0),
                        step: 3,
                        step_label: "3".into(),
                        bbox: None,
                        cached_path: std::cell::RefCell::new(None),
                    };
                    action.bbox = Editor::compute_bbox(&action);
                    all.push(action);
                }
            }
        }
    }
    all
}

/// The pixels `action` paints into an empty image: left, top, right, and
/// bottom pixel edges. `None` when it paints none.
fn raster_bounds(action: &Action) -> Option<[f32; 4]> {
    let mut img = image::RgbaImage::new(SIDE, SIDE);
    rasterize(&mut img, action, 1.0);
    let mut bounds: Option<[f32; 4]> = None;
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[3] == 0 {
            continue;
        }
        let (x, y) = (x as f32, y as f32);
        let b = bounds.get_or_insert([x, y, x + 1.0, y + 1.0]);
        *b = [
            b[0].min(x),
            b[1].min(y),
            b[2].max(x + 1.0),
            b[3].max(y + 1.0),
        ];
    }
    bounds
}

fn describe(action: &Action) -> String {
    format!(
        "{:?} width {} filled {} over {:?}",
        action.tool, action.width, action.filled, action.points
    )
}

#[test]
fn every_tool_paints_inside_the_footprint_a_rebuild_restores() {
    for action in actions() {
        let Some(ink) = raster_bounds(&action) else {
            continue;
        };
        let (x, y, w, h) = Editor::footprint(&action)
            .unwrap_or_else(|| panic!("{} paints but has no footprint", describe(&action)));
        let room = [x.floor(), y.floor(), (x + w).ceil(), (y + h).ceil()];
        assert!(
            ink[0] >= room[0] && ink[1] >= room[1] && ink[2] <= room[2] && ink[3] <= room[3],
            "{} paints {ink:?} outside its footprint {room:?}",
            describe(&action)
        );
    }
}

#[test]
fn the_canvas_draws_each_shape_where_the_saved_file_does() {
    for action in actions() {
        // The canvas draws these as text, as the raster's own patch, or
        // not at all.
        match action.tool {
            Tool::Pen
            | Tool::Highlight
            | Tool::Line
            | Tool::Arrow
            | Tool::Ellipse
            | Tool::Rect
            | Tool::Counter => {}
            Tool::Select | Tool::Text | Tool::Blur | Tool::Crop => continue,
        }
        let ink = raster_bounds(&action);
        for scale in [0.25, 0.5, 1.0, 2.0] {
            let mut rec = TriRecorder(Vec::new());
            tessellate(&action, scale, &mut rec);
            let drawn = rec.0.iter().fold(None, |bounds: Option<[f32; 4]>, t| {
                [(t[0], t[1]), (t[2], t[3]), (t[4], t[5])]
                    .into_iter()
                    .fold(bounds, |b, (x, y)| {
                        let (x, y) = (x / scale, y / scale);
                        Some(b.map_or([x, y, x, y], |b| {
                            [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
                        }))
                    })
            });
            match (ink, drawn) {
                (None, None) => {}
                (Some(ink), Some(drawn)) => assert!(
                    ink.iter().zip(drawn).all(|(i, d)| (i - d).abs() <= SLOP),
                    "{} at zoom {scale}: the canvas draws {drawn:?} where the file has {ink:?}",
                    describe(&action)
                ),
                _ => panic!(
                    "{} at zoom {scale}: the canvas draws {drawn:?} where the file has {ink:?}",
                    describe(&action)
                ),
            }
        }
    }
}

#[test]
fn an_undone_or_moved_action_leaves_no_ink_behind() {
    let base = image::RgbaImage::from_pixel(SIDE, SIDE, image::Rgba([30, 60, 90, 255]));
    let drawn = |actions: &mut [Action]| {
        let mut img = base.clone();
        Editor::replay_actions(&mut img, actions, 0);
        img
    };
    for action in actions() {
        let mut moved = action.clone();
        moved.points = Rc::new(
            action
                .points
                .iter()
                .map(|&(x, y)| (x + 17.0, y - 9.0))
                .collect(),
        );
        moved.bbox = Editor::compute_bbox(&moved);
        let edits = [
            ("an undo", Edit::Remove(0, action.clone()), vec![]),
            (
                "a move",
                Edit::Move(0, action.clone(), moved.clone()),
                vec![moved],
            ),
        ];
        for (what, edit, mut after) in edits {
            let mut composite = drawn(&mut [action.clone()]);
            // No region rebuilds the whole image, which leaves nothing.
            let Some(region) = Editor::edit_region(&edit) else {
                continue;
            };
            let (closed, mark) = Editor::dirty_plan(&mut after, region, SIDE as f32, SIDE as f32);
            Editor::apply_plan(&base, &mut composite, &mut after, closed, &mark);
            let want = drawn(&mut after);
            let stale = composite
                .enumerate_pixels()
                .filter(|&(x, y, p)| p != want.get_pixel(x, y))
                .count();
            assert_eq!(
                stale,
                0,
                "{what} of {} leaves {stale} pixels of its old ink",
                describe(&action)
            );
        }
    }
}
