//! Text fields and the folder field.

use std::path::PathBuf;
use std::rc::Rc;

use gpui::*;

use super::{push_button, ButtonStyle};
use crate::icons::{self, Icon};
use crate::theme;

/// What a text field shows besides its text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldState {
    /// At rest: fill and inner border only.
    Rest,
    /// Being edited: an accent focus ring and a caret after the text;
    /// `invalid` turns the ring destructive red.
    Editing { invalid: bool },
    /// Capturing a key combination: the focus ring, no caret, the text
    /// in secondary color (a prompt such as "Type shortcut").
    Recording,
}

/// The 3px focus ring around a control, drawn as a spread shadow so it
/// takes no layout space.
pub fn focus_ring(color: Rgba) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(3.),
        inset: false,
    }]
}

/// Single-line text field box, CONTROL_H tall: a dark fill with a 1px
/// inner border; editing adds the focus ring and the caret. The caller
/// has the text buffer and commits it (Return or blur).
pub fn text_field(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    state: FieldState,
) -> Stateful<Div> {
    let (border, ring) = match state {
        FieldState::Rest => (theme::FIELD_BORDER, None),
        FieldState::Editing { invalid: true } => {
            (theme::DANGER, Some(theme::alpha(theme::DANGER, 0.40)))
        }
        FieldState::Editing { invalid: false } | FieldState::Recording => {
            (theme::ACCENT, Some(theme::FOCUS_RING))
        }
    };
    let mut el = div()
        .id(id)
        .h(px(theme::CONTROL_H))
        .px(px(7.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(border)
        .text_size(px(theme::TEXT_BODY))
        .text_color(if state == FieldState::Recording {
            theme::FG_DIM
        } else {
            theme::FG
        })
        .cursor_text();
    if let Some(ring) = ring {
        el = el.shadow(focus_ring(ring));
    }
    el = el.child(text.into());
    if matches!(state, FieldState::Editing { .. }) {
        el = el.child(
            div()
                .w(px(1.5))
                .h(px(15.))
                .ml(px(1.))
                .flex_shrink_0()
                .bg(theme::ACCENT),
        );
    }
    el
}

/// A folder row control: a folder glyph and the path, middle-truncated
/// to `width` (the head and the folder name stay readable), then a
/// "Choose…" button that opens the platform's directory picker. A
/// chosen directory runs `on_pick(Ok(dir))`; a picker that fails to
/// open runs `on_pick(Err(message))`; a cancelled picker runs nothing.
pub fn folder_field(
    id: &'static str,
    shown: impl Into<SharedString>,
    width: f32,
    on_pick: impl Fn(Result<PathBuf, String>, &mut Window, &mut App) + 'static,
) -> Div {
    let on_pick = Rc::new(on_pick);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .w(px(width))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .child(icons::icon(Icon::Folder, theme::FG_DIM, theme::ICON_ROW))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_middle()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG_DIM)
                        .child(shown.into()),
                ),
        )
        .child(
            push_button(id, "Choose\u{2026}", ButtonStyle::Bordered).on_click(
                move |_, window, cx| {
                    let rx = cx.prompt_for_paths(PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Choose".into()),
                    });
                    let on_pick = on_pick.clone();
                    window
                        .spawn(cx, async move |cx| {
                            let picked = match rx.await {
                                Ok(Ok(Some(mut dirs))) => dirs.pop().map(Ok),
                                Ok(Ok(None)) | Err(_) => None,
                                Ok(Err(e)) => Some(Err(format!("folder picker: {e}"))),
                            };
                            if let Some(picked) = picked {
                                cx.update(|window, cx| on_pick(picked, window, cx)).ok();
                            }
                        })
                        .detach();
                },
            ),
        )
}
