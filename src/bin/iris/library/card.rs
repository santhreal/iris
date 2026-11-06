//! One card of the library grid: the thumbnail with its hover actions
//! and selection ring, and the caption under it.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::*;
use iris_lib::library::CaptureEntry;

use crate::icons::Icon;
use crate::theme;
use crate::widgets::{tip, OVERLAY_BUTTON};

use super::listing::Caption;
use super::{Library, CAPTION_GAP, CAPTION_H, CARD_W, THUMB_H};

/// The tile's corner radius, and the image's inside the mat.
pub(super) const THUMB_RADIUS: f32 = 8.0;
const IMAGE_RADIUS: f32 = 4.0;

/// Where a `w`x`h` capture's thumbnail sits in its card's CARD_W x
/// THUMB_H tile, as (left, top, width, height): the whole thumbnail at
/// half its pixel size, the 2x device scale the tile is sized for,
/// centered on whole device pixels so the image draws unresampled.
pub(super) fn thumb_rect(w: u32, h: u32) -> (f32, f32, f32, f32) {
    let (bw, bh) = ((2.0 * CARD_W) as u32, (2.0 * THUMB_H) as u32);
    let (tw, th) = iris_lib::library::thumb_size(w, h);
    (
        (bw.saturating_sub(tw) / 2) as f32 / 2.0,
        (bh.saturating_sub(th) / 2) as f32 / 2.0,
        tw as f32 / 2.0,
        th as f32 / 2.0,
    )
}

/// The selection ring: RING wide, RING_GAP outside the tile.
pub(super) const RING: f32 = 2.0;
pub(super) const RING_GAP: f32 = 2.0;

/// The hover actions: a row of ACTIONS buttons ACTION_GAP apart, set
/// ACTION_INSET in from the capture's bottom-right corner.
const ACTIONS: f32 = 3.0;
const ACTION_GAP: f32 = 4.0;
const ACTION_INSET: f32 = 6.0;

/// Where the hover actions' row goes in the tile, as (left, top), for a
/// capture drawn at `rect` (`thumb_rect`): ACTION_INSET in from its
/// bottom-right corner, so the row sits on the capture and not across
/// its edge and the mat. Along an axis where the capture is too small
/// to hold the row and both insets, the row centers on the capture.
/// Whole device pixels at 2x, as the capture is.
pub(super) fn actions_origin((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    let row = (
        ACTIONS * OVERLAY_BUTTON + (ACTIONS - 1.0) * ACTION_GAP,
        OVERLAY_BUTTON,
    );
    let place = |start: f32, len: f32, row: f32| {
        let at = if len >= row + 2.0 * ACTION_INSET {
            start + len - ACTION_INSET - row
        } else {
            start + (len - row) / 2.0
        };
        (2.0 * at).round() / 2.0
    };
    (place(x, w, row.0), place(y, h, row.1))
}

/// One shared transparent tile for a card whose thumbnail has not
/// decoded: minting a RenderImage per missing thumb per frame allocates
/// an atlas slot on every paint.
fn blank() -> Arc<RenderImage> {
    static BLANK: std::sync::LazyLock<Arc<RenderImage>> =
        std::sync::LazyLock::new(|| crate::widgets::render_image_from_rgba(1, 1, &[0, 0, 0, 0]));
    BLANK.clone()
}

impl Library {
    /// Entry `index`'s card at document position (`x`, `y`): `hover_amt`
    /// is its hover spring, 0 at rest and 1 under the pointer.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn card(
        &self,
        index: usize,
        entry: &Rc<CaptureEntry>,
        caption: &Caption,
        (x, y): (f32, f32),
        hover_amt: f32,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hovered = self.hovered == Some(index);
        // One Rc bump feeds every closure below; the alternative is a
        // PathBuf clone per closure per render.
        let entry = entry.clone();
        let (ix, iy, iw, ih) = thumb_rect(entry.width, entry.height);

        let mut thumb = div()
            .size_full()
            .relative()
            .rounded(px(THUMB_RADIUS))
            .overflow_hidden()
            .bg(theme::SURFACE)
            .child(
                img(ImageSource::Render(
                    self.thumb_cache
                        .get(&entry.path)
                        .cloned()
                        .unwrap_or_else(blank),
                ))
                .absolute()
                .left(px(ix))
                .top(px(iy))
                .w(px(iw))
                .h(px(ih))
                .rounded(px(IMAGE_RADIUS))
                .object_fit(ObjectFit::Fill),
            )
            // The hairline sits over the image: a border on the clipping
            // box paints under its children.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(px(THUMB_RADIUS))
                    .border_1()
                    .border_color(theme::HAIRLINE),
            );

        // Quick actions in the capture's bottom-right corner while the
        // pointer is on the card, fading in and out with its hover
        // spring: they cover the image, so they do not rest on it.
        if hovered || hover_amt > 0.0 {
            let (ax, ay) = actions_origin((ix, iy, iw, ih));
            thumb = thumb.child(
                div()
                    .absolute()
                    .left(px(ax))
                    .top(px(ay))
                    .flex()
                    .gap(px(ACTION_GAP))
                    .opacity(hover_amt)
                    .child(
                        crate::widgets::overlay_icon_button(("copy", index), Icon::Copy)
                            .tooltip(tip(
                                "Copy",
                                Some(crate::widgets::meta_shortcut!("C").into()),
                            ))
                            .on_click(cx.listener({
                                let entry = entry.clone();
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.copy_paths(vec![entry.path.clone()], cx);
                                }
                            })),
                    )
                    .child(
                        crate::widgets::overlay_icon_button(("reveal", index), Icon::Folder)
                            .tooltip(tip(crate::sys::reveal::LABEL, None))
                            .on_click({
                                let entry = entry.clone();
                                move |_, _, cx| {
                                    cx.stop_propagation();
                                    crate::sys::reveal::reveal(&entry.path);
                                }
                            }),
                    )
                    .child(
                        crate::widgets::overlay_icon_button(("trash", index), Icon::Trash)
                            .tooltip(tip("Move to Trash", Some("Delete".into())))
                            .on_click(cx.listener({
                                let entry = entry.clone();
                                move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.trash_paths(vec![entry.path.clone()], cx);
                                }
                            })),
                    ),
            );
        }

        let lift = {
            let mut s = theme::shadow_rest();
            s.color.a *= 0.6 + 0.9 * hover_amt;
            s.blur_radius = px(10. + 8.0 * hover_amt);
            s.offset.y = px(2. + 3.0 * hover_amt);
            s
        };
        // Every card's tile is the same CARD_W x THUMB_H, the capture
        // whole and centered on a matte, so rows and captions line up.
        let tile = div()
            .relative()
            .w(px(CARD_W))
            .h(px(THUMB_H))
            .flex_shrink_0()
            .rounded(px(THUMB_RADIUS))
            .shadow(vec![lift])
            .child(thumb)
            .children(selected.then(|| {
                let out = RING + RING_GAP;
                div()
                    .absolute()
                    .top(px(-out))
                    .left(px(-out))
                    .w(px(CARD_W + 2.0 * out))
                    .h(px(THUMB_H + 2.0 * out))
                    .rounded(px(THUMB_RADIUS + out))
                    .border(px(RING))
                    .border_color(theme::ACCENT)
            }));

        let caption_row = div()
            .id(("caption", index))
            .mt(px(CAPTION_GAP))
            .w(px(CARD_W))
            .h(px(CAPTION_H))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .whitespace_nowrap()
            .text_size(px(theme::TEXT_SMALL))
            .line_height(px(CAPTION_H))
            .font_features(theme::tabular())
            .child(
                div()
                    .text_color(theme::FG)
                    .font_weight(FontWeight::MEDIUM)
                    .child(caption.time.clone()),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(theme::FG_DIM)
                    .child(caption.dims.clone()),
            )
            .tooltip(tip(caption.name.clone(), None));

        div()
            .id(("card", index))
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(CARD_W))
            .flex()
            .flex_col()
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                if *hovering {
                    this.hovered = Some(index);
                } else if this.hovered == Some(index) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.click_card(index, ev, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.open_menu(index, ev.position, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, ev: &MouseDownEvent, _, _| {
                    this.drag_start = Some((index, ev.position.x.into(), ev.position.y.into()));
                    this.drag_fired = false;
                }),
            )
            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, window, cx| {
                if this.drag_fired || ev.pressed_button != Some(MouseButton::Left) {
                    return;
                }
                let Some((card, sx, sy)) = this.drag_start else {
                    return;
                };
                let (mx, my): (f32, f32) = (ev.position.x.into(), ev.position.y.into());
                if (mx - sx).hypot(my - sy) < 8.0 {
                    return;
                }
                this.drag_fired = true;
                // The index was captured at mousedown; a refresh that
                // shrank entries since then must not panic the drag.
                let Some(entry) = this.listing.entries().get(card) else {
                    this.drag_start = None;
                    return;
                };
                let paths: Vec<PathBuf> = if this.selected.contains(&entry.path) {
                    this.selected.clone()
                } else {
                    vec![entry.path.clone()]
                };
                // The cached RenderImage already holds the pixels
                // (BGRA; the swizzle is symmetric): no disk read or
                // PNG decode on the drag-start path.
                let icon = this.thumb_cache.get(&entry.path).and_then(|img| {
                    let size = img.size(0);
                    // One pass from the cache's BGRA bytes to the icon's
                    // RGBA, into the Arc the drag owns.
                    let src = img.as_bytes(0)?;
                    let rgba = iris_lib::pixel::map_to_vec(src, iris_lib::pixel::swap_rb);
                    Some(iris_lib::sys::dragcopy::DragIcon {
                        width: size.width.0 as u32,
                        height: size.height.0 as u32,
                        rgba: Arc::new(rgba),
                    })
                });
                if let Err(e) = crate::sys::window::start_file_drag(window, paths, icon) {
                    this.set_status(e, cx);
                }
            }))
            .child(tile)
            .child(caption_row)
    }
}
