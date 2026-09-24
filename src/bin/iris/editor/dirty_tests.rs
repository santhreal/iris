//! Regression tests for partial rebuild and dirty-region replay.

use std::rc::Rc;

use super::action::{Action, Tool};
use super::Editor;

fn base_action(tool: Tool) -> Action {
    Action {
        tool,
        color: "#ff0000",
        width: 2.0,
        points: Rc::new(Vec::new()),
        text: None,
        font_size: 16.0,
        filled: false,
        blur_patch: None,
        blur_rect: (0.0, 0.0, 0.0, 0.0),
        step: 0,
        step_label: "0".into(),
        bbox: None,
        cached_path: std::cell::RefCell::new(None),
    }
}

fn stroke(points: Vec<(f32, f32)>, bbox: (f32, f32, f32, f32)) -> Action {
    let mut a = base_action(Tool::Pen);
    a.points = Rc::new(points);
    a.bbox = Some(bbox);
    a
}

fn highlight(points: Vec<(f32, f32)>, bbox: (f32, f32, f32, f32)) -> Action {
    let mut a = base_action(Tool::Highlight);
    a.points = Rc::new(points);
    a.bbox = Some(bbox);
    a
}

fn blur(rect: (f32, f32, f32, f32)) -> Action {
    let mut a = base_action(Tool::Blur);
    a.blur_rect = rect;
    a.bbox = Some(rect);
    a
}

fn counter(at: (f32, f32), step: u32) -> Action {
    let mut a = base_action(Tool::Counter);
    a.points = Rc::new(vec![at]);
    a.step = step;
    a.step_label = step.to_string().into();
    let r = a.font_size * 0.9;
    a.bbox = Some((at.0 - r, at.1 - r, r * 2.0, r * 2.0));
    a
}

/// Full-rebuild reference: base plus every action in commit order.
fn full_rebuild(base: &image::RgbaImage, actions: &mut [Action]) -> image::RgbaImage {
    let mut img = base.clone();
    Editor::replay_actions(&mut img, actions, 0);
    img
}

/// The dirty path: plan against `region`, apply to a composite
/// that already holds the pre-edit render.
fn dirty_rebuild(
    base: &image::RgbaImage,
    composite: &mut image::RgbaImage,
    actions: &mut [Action],
    region: (f32, f32, f32, f32),
) {
    let (closed, mark) = Editor::dirty_plan(
        actions,
        region,
        composite.width() as f32,
        composite.height() as f32,
    );
    Editor::apply_plan(base, composite, actions, closed, &mark);
}

#[test]
fn dirty_rebuild_matches_full_rebuild() {
    // Two disjoint strokes: one top-left, one bottom-right.
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let b = stroke(vec![(30.0, 30.0), (38.0, 38.0)], (26.0, 26.0, 16.0, 16.0));
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

    let mut full = full_rebuild(&base, &mut [a.clone(), b.clone()]);

    // Remove b: the dirty region is its bbox; a must survive.
    let mut dirty = full.clone();
    let mut remaining = [a.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, (26.0, 26.0, 16.0, 16.0));
    full = full_rebuild(&base, &mut [a.clone()]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "dirty rebuild must equal full rebuild"
    );
    // The skipped stroke's pixels survived: they were never restored.
    assert_eq!(dirty.get_pixel(5, 2), full.get_pixel(5, 2));
}

#[test]
fn alpha_does_not_compound_outside_dirty() {
    // A highlight stroke whose footprint spills past the dirty
    // rect: replaying it over its own surviving ink doubles the
    // 0.35 alpha outside the rect. The region must grow to cover
    // the whole footprint.
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let h = highlight(vec![(4.0, 20.0), (36.0, 20.0)], (0.0, 15.0, 40.0, 10.0));
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

    let mut dirty = full_rebuild(&base, &mut [a.clone(), h.clone()]);
    // Remove a; h stays and intersects the dirty rect at its left.
    let mut remaining = [h.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

    let full = full_rebuild(&base, &mut [h.clone()]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "highlight ink outside the dirty rect must not be repainted over itself"
    );
}

#[test]
fn blur_resamples_restored_base() {
    // A blur whose rect spills past the dirty rect: replaying it
    // while the spill still holds the old blur output samples
    // blur-of-blur. The region must cover the blur's whole rect.
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let b = blur((0.0, 0.0, 40.0, 40.0));
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

    let mut dirty = full_rebuild(&base, &mut [a.clone(), b.clone()]);
    let mut remaining = [b.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

    let full = full_rebuild(&base, &mut [b.clone()]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "blur must resample restored base, not its own stale output"
    );
}

#[test]
fn transitive_pull_in_replays_chain() {
    // Removing a pulls in the blur that samples its ink; the
    // blur's footprint pulls in the counter stamped inside it.
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let b = blur((0.0, 0.0, 30.0, 30.0));
    let c = counter((20.0, 20.0), 1);
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

    let mut dirty = full_rebuild(&base, &mut [a.clone(), b.clone(), c.clone()]);
    let mut remaining = [b.clone(), c.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, (0.0, 0.0, 14.0, 6.0));

    let full = full_rebuild(&base, &mut [b.clone(), c.clone()]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "the closure must replay every action the region transitively touches"
    );
}

#[test]
fn renumbered_counter_repaints() {
    // Removing counter 1 renumbers counter 2 to 1: its ink
    // changes even though its footprint never intersected the
    // dirty rect.
    let c1 = counter((5.0, 5.0), 1);
    let c2 = counter((30.0, 30.0), 2);
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));

    let mut dirty = full_rebuild(&base, &mut [c1.clone(), c2.clone()]);
    let mut remaining = [c2.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, c1.bbox.unwrap());

    let mut c2_renumbered = c2.clone();
    c2_renumbered.step = 1;
    c2_renumbered.step_label = "1".into();
    let full = full_rebuild(&base, &mut [c2_renumbered]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "a renumbered counter must repaint its new digit"
    );
}

#[test]
fn no_intersecting_action_leaves_composite_alone() {
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let (region, mark) = Editor::replay_closure(&[a], (30.0, 30.0, 8.0, 8.0), 40.0, 40.0);
    assert!(mark.iter().all(|m| !*m), "a far-away region marks nothing");
    assert_eq!(region, (30.0, 30.0, 8.0, 8.0), "the region stays put");
}

#[test]
fn missing_bbox_replays_everything() {
    let mut a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    a.bbox = None;
    let (region, mark) = Editor::replay_closure(&[a], (30.0, 30.0, 8.0, 8.0), 40.0, 40.0);
    assert_eq!(mark, [true], "a bbox-less action conservatively replays");
    assert_eq!(
        region,
        (0.0, 0.0, 40.0, 40.0),
        "and widens to the full image"
    );
}

// WHY: restore_region bands the row copies once the image clears
// par_bands_mut's 1MB gate. The 40x40 tests above all run the
// serial path, so a band that drops or shifts a row would pass
// them. A 1024x512 image (2MB) splits into ~64-row bands; a dirty
// region spanning several bands must restore every covered row.
#[test]
fn banded_restore_region_matches_full_rebuild() {
    let base = image::RgbaImage::from_pixel(1024, 512, image::Rgba([10, 20, 30, 255]));
    // Two strokes: one top-left, one bottom-right. Removing the
    // bottom-right one dirties a region crossing band boundaries.
    let a = stroke(vec![(20.0, 20.0), (200.0, 20.0)], (0.0, 0.0, 220.0, 40.0));
    let b = stroke(
        vec![(100.0, 150.0), (900.0, 400.0)],
        (90.0, 140.0, 820.0, 280.0),
    );
    let mut dirty = full_rebuild(&base, &mut [a.clone(), b.clone()]);
    let mut remaining = [a.clone()];
    dirty_rebuild(&base, &mut dirty, &mut remaining, b.bbox.unwrap());
    let full = full_rebuild(&base, &mut [a.clone()]);
    assert_eq!(
        dirty.as_raw(),
        full.as_raw(),
        "banded restore_region must equal the full rebuild"
    );
}

/// The pre-closure algorithm: restore only the edit's rect, then
/// replay the suffix from the first intersecting action. Kept to
/// prove the regression tests above observe the bug it produced.
fn legacy_dirty(
    base: &image::RgbaImage,
    composite: &mut image::RgbaImage,
    actions: &mut [Action],
    region: (f32, f32, f32, f32),
) {
    let (iw, ih) = (composite.width(), composite.height());
    let Some(r) = Editor::clamp_region(region, iw, ih) else {
        return;
    };
    Editor::restore_region(base, composite, r);
    let first = actions
        .iter()
        .position(|a| Editor::footprint(a).is_none_or(|f| Editor::intersects(f, region)));
    if let Some(first) = first {
        Editor::replay_actions(composite, actions, first);
    }
}

#[test]
fn legacy_algorithm_is_observably_wrong() {
    // Alpha compounding: the old suffix replay repaints the
    // highlight over its own ink outside the dirty rect.
    let a = stroke(vec![(2.0, 2.0), (10.0, 2.0)], (0.0, 0.0, 14.0, 6.0));
    let h = highlight(vec![(4.0, 20.0), (36.0, 20.0)], (0.0, 15.0, 40.0, 10.0));
    let base = image::RgbaImage::from_pixel(40, 40, image::Rgba([10, 20, 30, 255]));
    let mut legacy = full_rebuild(&base, &mut [a.clone(), h.clone()]);
    legacy_dirty(
        &base,
        &mut legacy,
        &mut [a.clone(), h.clone()],
        (0.0, 0.0, 14.0, 6.0),
    );
    let full = full_rebuild(&base, &mut [h.clone()]);
    assert_ne!(
        legacy.as_raw(),
        full.as_raw(),
        "the old algorithm must compound alpha outside the dirty rect"
    );

    // Stale blur sampling: the old restore leaves the blur's own
    // output under the spill, so the resample blurs blur.
    let b = blur((0.0, 0.0, 40.0, 40.0));
    let mut legacy = full_rebuild(&base, &mut [a.clone(), b.clone()]);
    legacy_dirty(
        &base,
        &mut legacy,
        &mut [a.clone(), b.clone()],
        (0.0, 0.0, 14.0, 6.0),
    );
    let full = full_rebuild(&base, &mut [b.clone()]);
    assert_ne!(
        legacy.as_raw(),
        full.as_raw(),
        "the old algorithm must sample its own stale blur output"
    );
}
