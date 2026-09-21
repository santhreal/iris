//! Shared chrome widgets: one implementation per control, used by
//! every surface. The register lives in theme.rs; glyphs live in
//! icons.rs. Buttons are ghost by default — a hover wash, no fill —
//! because that is how Apple's chrome reads; filled is reserved for
//! the single primary action.

use gpui::*;

use crate::{icons, theme};
use icons::Icon;

/// Text button. Ghost (hover wash) unless `primary`, which is the
/// one filled action on a surface.
pub fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    let base = div()
        .id(ElementId::Name(id.into()))
        .h(px(28.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(theme::RADIUS_SM))
        .text_sm()
        .cursor_pointer()
        .child(label);
    if primary {
        base.bg(theme::ACCENT)
            .text_color(theme::ACCENT_INK)
            .font_weight(FontWeight::SEMIBOLD)
            .active(|s| s.bg(theme::alpha(theme::ACCENT, 0.82)))
    } else {
        base.text_color(theme::FG_DIM)
            .hover(|s| s.bg(theme::SURFACE_HOVER).text_color(theme::FG))
            .active(|s| s.bg(theme::SURFACE_PRESS))
    }
}

/// Text button with a trailing glyph (dropdown chevrons).
pub fn button_with_icon(
    id: &'static str,
    label: &'static str,
    glyph: Icon,
    primary: bool,
) -> Stateful<Div> {
    let color = if primary { theme::ACCENT_INK } else { theme::FG_DIM };
    button(id, label, primary).child(icons::icon(glyph, color, 10.0))
}

/// Icon-only button, `size`px square. `active` fills the chip and
/// inverts the glyph; otherwise ghost with a hover wash.
pub fn icon_button(id: impl Into<ElementId>, glyph: Icon, active: bool, size: f32) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(size))
        .h(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(10.))
        .cursor_pointer()
        .flex_shrink_0()
        .bg(if active { theme::FG } else { theme::alpha(theme::FG, 0.0) })
        .hover(move |s| if active { s } else { s.bg(theme::SURFACE_HOVER) })
        .active(move |s| if active { s } else { s.bg(theme::SURFACE_PRESS) })
        .child(icons::icon(
            glyph,
            if active { theme::ACCENT_INK } else { theme::FG_DIM },
            size * 0.5,
        ))
}

/// Small icon button for card overlays: 26px on a frosted chip.
pub fn overlay_icon_button(id: impl Into<ElementId>, glyph: Icon) -> Stateful<Div> {
    overlay_icon_button_active(id, glyph, false)
}

/// Small icon button for card overlays, with optional active/highlight state.
pub fn overlay_icon_button_active(id: impl Into<ElementId>, glyph: Icon, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .w(px(26.))
        .h(px(26.))
        .rounded(px(theme::RADIUS_SM))
        .bg(if active {
            theme::FG
        } else {
            theme::alpha(theme::BG_ELEV, 0.92)
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| {
            if active {
                s
            } else {
                s.bg(theme::SURFACE_HOVER)
            }
        })
        .child(icons::icon(
            glyph,
            if active {
                theme::ACCENT_INK
            } else {
                theme::FG
            },
            14.0,
        ))
}

/// Menu geometry, fixed so callers can place and hit-test a menu
/// without measuring: rows are exactly MENU_ROW_H tall and the panel
/// is MENU_W wide. A menu of `rows` is MENU_ROW_H*rows + 10 tall.
pub const MENU_W: f32 = 152.0;
pub const MENU_ROW_H: f32 = 28.0;

/// Dropdown menu surface. Caller fills it with `menu_row`s.
pub fn menu() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(10.))
        .bg(theme::alpha(theme::BG_ELEV, 0.96))
        .border_1()
        .border_color(theme::HAIRLINE)
        .shadow(theme::shadow_float())
        .py(px(4.))
        .w(px(MENU_W))
}

pub fn menu_row(id: &'static str, label: &'static str) -> Stateful<Div> {
    menu_row_owned(ElementId::Name(id.into()), label)
}

/// A menu row with a runtime label (dropdown options, file names).
/// `&'static str` stores without an allocation; a `String` moves
/// into the Arc.
pub fn menu_row_owned(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(MENU_ROW_H))
        .px(px(12.))
        .flex()
        .items_center()
        .text_sm()
        .text_color(theme::FG)
        .cursor_pointer()
        // A menu floats above other interactive surfaces; without
        // this the press falls through to whatever is beneath (the
        // toast card's drag tracking, the editor's topbar buttons).
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .hover(|s| s.bg(theme::SURFACE_HOVER))
        .child(label.into())
}

/// A dropdown field: a button showing the current value with a
/// chevron. The parent owns the open state and builds the option
/// menu beneath it when open (see `dropdown_row` in settings).
pub fn dropdown(id: impl Into<ElementId>, current: impl Into<SharedString>, open: bool) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(6.))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(if open { theme::ACCENT } else { theme::HAIRLINE })
        .text_sm()
        .text_color(theme::FG)
        .cursor_pointer()
        .child(current.into())
        .child(icons::icon(Icon::ChevronDown, theme::FG_DIM, 10.0))
}

/// macOS-style toggle: 40x22 pill, 16px knob. The knob is white in
/// both states; the track carries the state.
pub fn toggle(id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(40.))
        .h(px(22.))
        .rounded_full()
        .bg(if on { theme::ACCENT } else { theme::alpha(theme::FG, 0.16) })
        .cursor_pointer()
        .child(
            div()
                .w(px(16.))
                .h(px(16.))
                .rounded_full()
                .bg(if on { theme::ACCENT_INK } else { theme::FG })
                // macOS toggle knobs carry a small drop shadow so they
                // read as a raised control, not a flat disc.
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::hsla(0.0, 0.0, 0.0, 0.28),
                    offset: gpui::point(px(0.), px(1.)),
                    blur_radius: px(2.),
                    spread_radius: px(0.),
                }])
                .ml(if on { px(21.) } else { px(3.) })
                .mt(px(3.)),
        )
}

/// Single-line text field box. Resting state has no border, like a
/// macOS form field; editing draws a soft ring.
pub fn text_field(id: impl Into<ElementId>, text: String, active: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(if active {
            theme::alpha(theme::FG, 0.4)
        } else {
            theme::alpha(theme::FG, 0.0)
        })
        .text_sm()
        .text_color(theme::FG)
        .cursor_text()
        .child(text)
}

/// GPUI paints an `Arc<Image>` by decoding it into a `RenderImage` and
/// caching both halves forever: `fetch_asset` never evicts the app-wide
/// decode cache, and the sprite-atlas tile (keyed by the RenderImage's
/// unique id) is only removed by an explicit `drop_image`, which the
/// `ImageSource::Image` path never calls. Feeding unique images — every
/// capture's frame, thumb and loupe — therefore leaks GPU memory
/// without bound. The escape hatch is `ImageSource::Render`: the app
/// constructs the `RenderImage` itself, so it can free the atlas tile
/// with `cx.drop_image` when the surface closes. The pixel copy is also
/// cheaper than the old path (BMP encode + decode round trip).
///
/// `rgba` is a tightly packed RGBA8 buffer; GPUI's atlas wants BGRA.
/// RGBA→BGRA in place, one u32 per pixel: the rotate form vectorizes;
/// a byte-wise swap does not.
#[inline]
pub(crate) fn swizzle_rgba_bgra(chunk: &mut [u8]) {
    for px in chunk.chunks_exact_mut(4) {
        let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
        let bgr = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
        px.copy_from_slice(&bgr.to_le_bytes());
    }
}

pub fn render_image_from_rgba(width: u32, height: u32, rgba: &[u8]) -> std::sync::Arc<gpui::RenderImage> {
    // Uninit capacity, not a zeroed vec: the fused copy+swizzle writes
    // every byte, and a multi-MB memset before a multi-MB fill is a
    // wasted pass. On failure the buffer drops without being read.
    let mut data: Vec<u8> = Vec::with_capacity(rgba.len());
    #[allow(clippy::uninit_vec)]
    unsafe { data.set_len(rgba.len()) };
    // Fused copy+swizzle, banded across threads once the buffer is
    // large enough to pay for the spawn (the 152px loupe, rebuilt
    // every mousemove, stays inline).
    let row = width as usize * 4;
    iris_lib::par::par_bands_mut(&mut data, row, |dst, start| {
        dst.copy_from_slice(&rgba[start..start + dst.len()]);
        swizzle_rgba_bgra(dst);
    });
    let buf = image::RgbaImage::from_raw(width, height, data).expect("rgba buffer size");
    std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(buf)]))
}

/// Same as `render_image_from_rgba` but takes ownership of the buffer
/// and swizzles in place: no second copy of a multi-MB frame. The
/// overlay uses this so the frozen frame's only CPU copy IS the
/// RenderImage's buffer.
pub fn render_image_from_rgba_owned(width: u32, height: u32, mut rgba: Vec<u8>) -> std::sync::Arc<gpui::RenderImage> {
    let row = width as usize * 4;
    iris_lib::par::par_bands_mut(&mut rgba, row, |band, _| {
        swizzle_rgba_bgra(band);
    });
    let buf = image::RgbaImage::from_raw(width, height, rgba).expect("rgba buffer size");
    std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(buf)]))
}
/// Same construction from encoded PNG bytes (thumbs, editor base).
pub fn render_image_from_png(bytes: &[u8]) -> Option<std::sync::Arc<gpui::RenderImage>> {
    let mut data = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .ok()?
        .into_rgba8();
    // Same banding as the rgba path: a 4K editor base is a 33MB
    // swizzle, too big for one thread.
    let row = data.width() as usize * 4;
    iris_lib::par::par_bands_mut(data.as_mut(), row, |band, _| {
        swizzle_rgba_bgra(band);
    });
    Some(std::sync::Arc::new(gpui::RenderImage::new([image::Frame::new(data)])))
}

/// Free a `RenderImage`'s sprite-atlas tile across every window.
/// Release between frames (event handlers) or via `cx.defer` from
/// render; dropping mid-paint blanks the frame.
pub fn release_render(image: &std::sync::Arc<gpui::RenderImage>, cx: &mut App) {
    cx.drop_image(image.clone(), None);
}

/// Transient status line, `left` px from the window's left edge,
/// bottom-aligned, frosted.
pub fn status_pill(text: &str, left: f32) -> Div {
    div()
        .absolute()
        .bottom(px(10.))
        .left(px(left))
        .px(px(10.))
        .py(px(5.))
        .rounded(px(theme::RADIUS_SM))
        .bg(theme::alpha(theme::BG_ELEV, 0.95))
        .shadow(theme::shadow_float())
        .text_xs()
        .text_color(theme::FG_DIM)
        .child(text.to_string())
}

/// One window-control dot (close/minimize/zoom). 12px, brightens on
/// hover, darkens on press.
fn win_dot(id: &'static str, color: Rgba) -> Stateful<Div> {
    div()
        .id(ElementId::Name(id.into()))
        .w(px(12.))
        .h(px(12.))
        .rounded_full()
        .bg(color)
        .cursor_pointer()
        .hover(move |s| s.bg(theme::alpha(color, 0.85)))
        .active(move |s| s.bg(theme::alpha(color, 0.65)))
        // A press on a window control must not start a title-bar drag.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// The three window controls, macOS order and register: close,
/// minimize, zoom (fullscreen). Used by window_frame so every Normal
/// window offers the same controls in the same place.
pub fn traffic_lights(
    on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_min: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_zoom: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(win_dot("win-close", theme::WIN_CLOSE).on_click(on_close))
        .child(win_dot("win-min", theme::WIN_MIN).on_click(on_min))
        .child(win_dot("win-zoom", theme::WIN_ZOOM).on_click(on_zoom))
}

/// The shared Normal-window scaffold: rounded root (the window is
/// transparent; this root's 12px corners are the window's shape),
/// a 56px toolbar with traffic lights, a semibold title and the
/// caller's right-side cluster, then the content. Double-clicking
/// the toolbar toggles fullscreen; pressing it drags the window via
/// _NET_WM_MOVERESIZE, because client-side decorations leave the WM
/// nothing to grab. `class` is the window's unique WM_CLASS, how
/// the move request finds its XID.
pub fn window_frame(
    title: &'static str,
    class: SharedString,
    right: Vec<AnyElement>,
    content: impl IntoElement,
) -> Div {
    let mut cluster = div().flex().items_center().gap(px(6.));
    for el in right {
        cluster = cluster.child(el);
    }
    // Buttons and fields in the cluster handle their own presses.
    cluster = cluster.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
    div()
        .size_full()
        .rounded(px(12.))
        .overflow_hidden()
        .bg(theme::BG)
        .flex()
        .flex_col()
        .child(
            div()
                .id("frame-toolbar")
                .h(px(56.))
                .flex()
                .items_center()
                .justify_between()
                .px(px(20.))
                .border_b_1()
                .border_color(theme::HAIRLINE)
                .on_mouse_down(MouseButton::Left, move |ev, window, _| {
                    if ev.click_count == 2 {
                        window.toggle_fullscreen();
                        return;
                    }
                    // Single press: hand the drag to the WM. Root
                    // coordinates are window origin + local position.
                    let origin = window.bounds().origin;
                    crate::xwin::begin_wm_move(
                        class.to_string(),
                        (f32::from(origin.x) + f32::from(ev.position.x)) as i32,
                        (f32::from(origin.y) + f32::from(ev.position.y)) as i32,
                    );
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .child(traffic_lights(
                            |_, window, _| window.remove_window(),
                            |_, window, _| window.minimize_window(),
                            |_, window, _| window.toggle_fullscreen(),
                        ))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::FG)
                                .child(title),
                        ),
                )
                .child(cluster),
        )
        .child(content)
}

/// A keyboard-shortcuts sheet: centered card over a dim layer, rows
/// of action / key combination. The surface owns the open state and
/// closes on Esc or a press on the dim layer.
pub fn shortcuts_sheet(rows: Vec<(&'static str, String)>) -> Stateful<Div> {
    let mut list = div().flex().flex_col().gap(px(2.)).mt(px(10.));
    for (action, keys) in rows {
        list = list.child(
            div()
                .h(px(30.))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(24.))
                .child(div().text_sm().text_color(theme::FG).child(action))
                .child(
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(theme::FIELD_BG)
                        .text_xs()
                        .text_color(theme::FG_DIM)
                        .child(keys),
                ),
        );
    }
    div()
        .id("sheet-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(theme::alpha(theme::BG, 0.55))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .flex()
                .flex_col()
                .p(px(20.))
                .rounded(px(12.))
                .bg(theme::BG_ELEV)
                .shadow(theme::shadow_float())
                // Presses on the card must not reach the scrim's
                // close handler.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::FG)
                        .child("Keyboard shortcuts"),
                )
                .child(list),
        )
}

// WHY: the class closed here is "the swizzle scrambles channels":
// every capture, thumbnail, and RenderImage passes through it, so a
// wrong lane silently recolors or drops alpha across the whole app.
// The involution case matters because the same function is applied
// symmetrically (RGBA->BGRA on the way in, BGRA->RGBA on the way out).
// Not covered: the banding that calls it, which par.rs tests own.
#[cfg(test)]
mod tests {
    use super::swizzle_rgba_bgra;

    #[test]
    fn swizzle_swaps_red_and_blue_keeps_green_alpha() {
        let mut px = vec![0x11, 0x22, 0x33, 0x44];
        swizzle_rgba_bgra(&mut px);
        assert_eq!(px, vec![0x33, 0x22, 0x11, 0x44]);
    }

    #[test]
    fn swizzle_is_an_involution() {
        let original: Vec<u8> = (0..64).map(|i| (i * 37 + 11) as u8).collect();
        let mut buf = original.clone();
        swizzle_rgba_bgra(&mut buf);
        swizzle_rgba_bgra(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn swizzle_handles_many_pixels() {
        // A multi-pixel buffer: each pixel swizzles independently, no
        // cross-lane bleed.
        let mut buf = vec![
            0xAA, 0xBB, 0xCC, 0xDD, // px0
            0x01, 0x02, 0x03, 0x04, // px1
            0xFF, 0x00, 0x80, 0x7F, // px2
        ];
        swizzle_rgba_bgra(&mut buf);
        assert_eq!(
            buf,
            vec![
                0xCC, 0xBB, 0xAA, 0xDD, // px0
                0x03, 0x02, 0x01, 0x04, // px1
                0x80, 0x00, 0xFF, 0x7F, // px2
            ]
        );
    }
}
