//! Vector icons: one stroke-weighted set for every chrome glyph.
//!
//! Text labels and emoji do not ship on chrome. Every icon is drawn
//! into an 18x18 logical box (scaled to the element bounds) with the
//! same 1.6px-equivalent stroke, using the path-tessellation helpers
//! the editor's vector markup already uses. One owner for the helpers:
//! editor.rs paints through these too.

use gpui::*;

mod tessellate;

pub(crate) use tessellate::{
    paint_icon, push_disc, push_ellipse_fill, push_filled_triangle, push_quad, push_rect_fill,
    push_ring, push_segment, TriRecorder,
};

// Some variants are only drawn by the Linux recording chip; the enum is
// a shared vocabulary, so the unused-on-Windows variants stay.
#[allow(dead_code)]
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
    Alert,
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

/// Tessellated icon triangles recorded at origin (0,0), keyed on
/// (kind, size): geometry no longer depends on the paint origin, so a
/// window drag reuses one entry per icon instead of minting one per
/// pixel moved. Triangles are re-stamped at each paint's offset; the
/// color is supplied at paint time and is not part of the key.
/// The one icon-geometry map: two separate statics here meant stores
/// went into a map reads never consulted, so every paint re-tessellated
/// and the "cache" was a bounded leak. Eviction is FIFO, not flush-all.
fn icon_cache() -> &'static parking_lot::Mutex<IconCache> {
    static CACHE: std::sync::LazyLock<parking_lot::Mutex<IconCache>> =
        std::sync::LazyLock::new(|| parking_lot::Mutex::new(IconCache::default()));
    &CACHE
}

#[derive(Default)]
struct IconCache {
    map: std::collections::HashMap<(u8, u32), std::sync::Arc<Vec<[f32; 6]>>>,
    order: std::collections::VecDeque<(u8, u32)>,
}

pub(super) fn cached_tris(kind: Icon, s: f32) -> Option<std::sync::Arc<Vec<[f32; 6]>>> {
    icon_cache()
        .lock()
        .map
        .get(&(kind as u8, s.to_bits()))
        .cloned()
}

pub(super) fn store_tris(kind: Icon, s: f32, tris: Vec<[f32; 6]>) -> std::sync::Arc<Vec<[f32; 6]>> {
    let mut cache = icon_cache().lock();
    let key = (kind as u8, s.to_bits());
    let tris = std::sync::Arc::new(tris);
    if !cache.map.contains_key(&key) {
        cache.order.push_back(key);
    }
    cache.map.insert(key, tris.clone());
    while cache.map.len() > 256 {
        if let Some(old) = cache.order.pop_front() {
            cache.map.remove(&old);
        } else {
            break;
        }
    }
    tris
}
