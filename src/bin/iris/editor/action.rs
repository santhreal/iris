//! Action definitions, tool types, and geometry helpers.

use std::{rc::Rc, sync::Arc};

use gpui::*;

use super::Editor;
use crate::icons::Icon;

/// Markup colors are user content, not UI chrome: the monochrome
/// register governs the interface, the palette governs what you draw.
pub(crate) const COLORS: [&str; 11] = [
    "#f5f5f7", "#9c9ca2", "#6b6b71", "#3a3a3f", "#141416", "#ff453a", "#ff9f0a", "#ffd60a",
    "#30d158", "#0a84ff", "#bf5af2",
];

pub(crate) fn hex_rgba(hex: &str) -> Rgba {
    let v = u32::from_str_radix(&hex[1..], 16).unwrap_or(0xffffff);
    Rgba {
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

/// Whole-image transforms (rotate/flip), applied to base+composite.
#[derive(Clone, Copy)]
pub(crate) enum Transform {
    Rot90,
    FlipH,
    FlipV,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Tool {
    Select,
    Pen,
    Line,
    Arrow,
    Ellipse,
    Rect,
    Text,
    Highlight,
    Blur,
    Crop,
    Counter,
}

pub(crate) const TOOLS: [(Tool, Icon, &str); 11] = [
    (Tool::Select, Icon::Cursor, "Select"),
    (Tool::Pen, Icon::Pen, "Pen"),
    (Tool::Line, Icon::Line, "Line"),
    (Tool::Arrow, Icon::Arrow, "Arrow"),
    (Tool::Ellipse, Icon::Ellipse, "Ellipse"),
    (Tool::Rect, Icon::Rect, "Rectangle"),
    (Tool::Text, Icon::Text, "Text"),
    (Tool::Highlight, Icon::Highlight, "Highlight"),
    (Tool::Blur, Icon::Blur, "Blur"),
    (Tool::Crop, Icon::Crop, "Crop"),
    (Tool::Counter, Icon::Counter, "Counter"),
];

#[derive(Clone)]
pub(crate) struct Action {
    pub(crate) tool: Tool,
    pub(crate) color: &'static str,
    pub(crate) width: f32,
    /// Stroke points behind Rc: every Action clone (commit's
    /// Edit::Add, undo/redo apply, the move-drag snapshot) is a
    /// refcount bump instead of a copy of every point. Mutations go
    /// through Rc::make_mut, which copies only while a snapshot
    /// still shares the buffer.
    pub(crate) points: Rc<Vec<(f32, f32)>>,
    pub(crate) text: Option<SharedString>,
    pub(crate) font_size: f32,
    /// Rect/Ellipse fill: when set the shape paints its interior, not
    /// just the outline.
    pub(crate) filled: bool,
    /// Pixelated patch for blur actions, computed at commit time from
    /// the composite below this action.
    pub(crate) blur_patch: Option<Arc<RenderImage>>,
    pub(crate) blur_rect: (f32, f32, f32, f32),
    /// Counter tool: the step number shown in the badge.
    pub(crate) step: u32,
    /// Its display string, cached at commit: to_string() per counter
    /// per frame was an allocation a frame.
    pub(crate) step_label: SharedString,
    /// Bounding box in image px, computed at commit and translated by
    /// move drags: hit-testing every committed action per click and
    /// the selected outline per frame would otherwise rescan every
    /// stroke's points each time.
    pub(crate) bbox: Option<(f32, f32, f32, f32)>,
    /// The tessellated paint triangles at stage origin (0,0), keyed on
    /// (scale, points fingerprint): building them per frame
    /// re-tessellates 20-vertex discs for every segment of every
    /// stroke, while a re-stamp is one offset per vertex. Origin is
    /// not part of the key, so a pan drag reuses the same geometry
    /// instead of re-tessellating every action every frame. Painted
    /// on the UI thread only, so a RefCell suffices. The fingerprint
    /// (len, first, last) catches every mutation that exists: strokes
    /// only grow, moves shift every point.
    pub(crate) cached_path: std::cell::RefCell<Option<PathCache>>,
}

/// Cached paint geometry: (scale, point count, first point, last
/// point, triangles at stage origin). The first four are the key.
pub(crate) type PathCache = (f32, usize, (f32, f32), (f32, f32), Rc<Vec<[f32; 6]>>);

pub(crate) struct TextEntry {
    pub(crate) point: (f32, f32),
    pub(crate) buffer: String,
    pub(crate) caret: SharedString,
    pub(crate) buffer_str: SharedString,
}

/// Stroke, highlight and text scale with the capture's pixel size, so a
/// stroke reads the same on a 400px region grab and a 5K full-screen shot.
/// Stroke width stops: multipliers over the image-relative base.
pub(crate) const STROKE_MULT: [f32; 3] = [0.6, 1.0, 1.8];

pub(crate) fn stroke_base(w: u32) -> f32 {
    (w as f32 * 0.003).clamp(2.0, 16.0)
}
pub(crate) fn highlight_width(w: u32) -> f32 {
    (w as f32 * 0.018).clamp(16.0, 64.0)
}
pub(crate) fn text_size(w: u32) -> f32 {
    (w as f32 * 0.016).clamp(14.0, 48.0)
}

impl Editor {
    pub(crate) fn new_action(&self, p: (f32, f32)) -> Action {
        let step = self.next_step();
        Action {
            tool: self.tool,
            color: self.color,
            width: if self.tool == Tool::Highlight {
                highlight_width(self.img_w())
            } else {
                stroke_base(self.img_w()) * STROKE_MULT[self.stroke as usize]
            },
            points: Rc::new(vec![p]),
            text: None,
            font_size: text_size(self.img_w()),
            filled: self.fill,
            blur_patch: None,
            blur_rect: (0.0, 0.0, 0.0, 0.0),
            step,
            step_label: SharedString::from(step.to_string()),
            bbox: None,
            cached_path: std::cell::RefCell::new(None),
        }
    }

    /// Bounding box of one action in image pixels, computed at commit.
    /// Stored on the action: hit-testing and the selection outline
    /// read it instead of rescanning every point.
    pub(crate) fn compute_bbox(action: &Action) -> Option<(f32, f32, f32, f32)> {
        match action.tool {
            Tool::Text => {
                let p = action.points.first()?;
                let text = action.text.as_ref()?;
                let w = text.chars().count() as f32 * action.font_size * 0.6;
                Some((
                    p.0,
                    p.1 - action.font_size,
                    w.max(8.0),
                    action.font_size * 1.25,
                ))
            }
            Tool::Blur => Some(action.blur_rect),
            Tool::Counter => {
                let p = action.points.first()?;
                let r = action.font_size * 0.9;
                Some((p.0 - r, p.1 - r, r * 2.0, r * 2.0))
            }
            _ => {
                if action.points.is_empty() {
                    return None;
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in action.points.iter() {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                let pad = action.width / 2.0 + 2.0;
                Some((
                    x0 - pad,
                    y0 - pad,
                    (x1 - x0) + 2.0 * pad,
                    (y1 - y0) + 2.0 * pad,
                ))
            }
        }
    }

    /// Topmost action whose bbox contains `p`, for the Select tool.
    pub(crate) fn hit_action(&self, p: (f32, f32)) -> Option<usize> {
        for (i, action) in self.actions.borrow().iter().enumerate().rev() {
            let Some((x, y, w, h)) = action.bbox else {
                continue;
            };
            if p.0 >= x && p.0 <= x + w && p.1 >= y && p.1 <= y + h {
                return Some(i);
            }
        }
        None
    }
}
