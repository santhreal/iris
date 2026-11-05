//! The tooltip: one dark pill with a caption label and the shortcut in
//! secondary color, shown by GPUI after the pointer rests on its element
//! for 500 ms.

use gpui::*;

use crate::theme;

/// A tooltip builder for GPUI's `.tooltip(...)`: `label`, then
/// `shortcut` when there is one.
///
/// ```ignore
/// icon_button("undo", Icon::Undo, false, 28.0).tooltip(tip("Undo", Some("Ctrl+Z".into())))
/// ```
pub fn tip(
    label: impl Into<SharedString>,
    shortcut: Option<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    move |_, cx| {
        let tooltip = Tooltip {
            label: label.clone(),
            shortcut: shortcut.clone(),
        };
        cx.new(|_| tooltip).into()
    }
}

struct Tooltip {
    label: SharedString,
    shortcut: Option<SharedString>,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // GPUI places the tooltip at the pointer; the margin keeps the
        // pill clear of the cursor glyph.
        div()
            .pl(px(6.))
            .pt(px(16.))
            .child(tip_pill(self.label.clone(), self.shortcut.as_deref()))
    }
}

/// The space between a control and its name drawn beside it.
pub const NAME_GAP: f32 = 4.0;

/// The name of the control whose hover group is `group`, drawn beside
/// it while the pointer is on it, as tall as the control's `height`:
/// for a surface with no room for a tooltip at the pointer. The caller
/// places it with `left` or `right`, NAME_GAP past the control.
pub fn hover_name(group: &'static str, label: impl Into<SharedString>, height: f32) -> Div {
    tip_pill(label.into(), None)
        .absolute()
        .top_0()
        .h(px(height))
        .py_0()
        .opacity(0.)
        .group_hover(group, |s| s.opacity(1.))
}

/// The tooltip's pill: `label`, then `shortcut` in secondary color when
/// there is one.
fn tip_pill(label: SharedString, shortcut: Option<&str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(theme::BG_ELEV)
        .border_1()
        .border_color(theme::HAIRLINE)
        .shadow(theme::shadow_float())
        .font_family(theme::FONT)
        .text_size(px(theme::TEXT_SMALL))
        .whitespace_nowrap()
        .text_color(theme::FG)
        .child(label)
        .children(shortcut.map(|keys| {
            div()
                .text_color(theme::FG_DIM)
                .child(super::shortcut_text(keys))
        }))
}
