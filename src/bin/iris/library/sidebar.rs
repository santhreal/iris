//! The app window's sidebar: the library, then the settings panes under
//! a section header. A click shows its page; the row of the page shown
//! is filled.

use gpui::*;

use super::{Library, Page};
use crate::icons::{self, Icon};
use crate::settings::Pane;
use crate::theme;

/// The space between a sidebar row's icon and its label, and the
/// icon's size.
const ICON_GAP: f32 = 8.0;
const ICON: f32 = 16.0;

impl Library {
    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> Div {
        let shown = match (self.page, &self.settings) {
            (Page::Settings, Some(s)) => Some(s.read(cx).pane()),
            _ => None,
        };
        let library = row("sidebar-library", Icon::Library, "Library", shown.is_none())
            .on_click(cx.listener(|this, _, window, cx| this.set_page(Page::Library, window, cx)));
        let mut list = div()
            .flex()
            .flex_col()
            .px(px(theme::SIDEBAR_PAD))
            .child(library)
            .child(
                div()
                    .h(px(theme::SIDEBAR_ROW_H))
                    .mt(px(theme::SIDEBAR_SECTION_GAP))
                    .pl(px(theme::SIDEBAR_ROW_PAD))
                    .flex()
                    .items_center()
                    .text_size(px(theme::TEXT_SMALL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::FG_DIM)
                    .child("Settings"),
            );
        for (i, pane) in Pane::ALL.into_iter().enumerate() {
            list = list.child(
                row(
                    ("sidebar-pane", i),
                    pane.icon(),
                    pane.label(),
                    shown == Some(pane),
                )
                .on_click(cx.listener(move |this, _, window, cx| this.show_pane(pane, window, cx))),
            );
        }
        list
    }

    /// Show the settings page on `pane`.
    fn show_pane(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        self.set_page(Page::Settings, window, cx);
        if let Some(settings) = &self.settings {
            settings.update(cx, |s, cx| {
                if s.select_pane(pane) {
                    cx.notify();
                }
            });
        }
    }
}

/// One sidebar row: the icon in the accent, the label, and the fill of
/// the page shown.
fn row(id: impl Into<ElementId>, icon: Icon, label: &'static str, selected: bool) -> Stateful<Div> {
    let mut row = div()
        .id(id)
        .h(px(theme::SIDEBAR_ROW_H))
        .flex_shrink_0()
        .px(px(theme::SIDEBAR_ROW_PAD))
        .flex()
        .items_center()
        .gap(px(ICON_GAP))
        .rounded(px(theme::RADIUS_CONTROL))
        .child(icons::icon(icon, theme::ACCENT, ICON))
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(theme::FG)
                .whitespace_nowrap()
                .child(label),
        );
    if selected {
        row = row.bg(theme::SIDEBAR_SELECTED);
    }
    row
}
