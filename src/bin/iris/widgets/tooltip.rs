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
        div().pl(px(6.)).pt(px(16.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(theme::RADIUS_CONTROL))
                .bg(theme::alpha(theme::BG_ELEV, 0.98))
                .border_1()
                .border_color(theme::HAIRLINE)
                .shadow(theme::shadow_float())
                .font_family(theme::FONT)
                .text_size(px(theme::TEXT_SMALL))
                .whitespace_nowrap()
                .text_color(theme::FG)
                .child(self.label.clone())
                .children(
                    self.shortcut
                        .clone()
                        .map(|keys| div().text_color(theme::FG_DIM).child(keys)),
                ),
        )
    }
}
