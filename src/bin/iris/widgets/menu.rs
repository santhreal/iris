//! Menus and the pop-up button. A menu is a floating panel of 24px rows
//! with a checkmark column for the selected value and a right-aligned
//! shortcut column; a pop-up button shows the current value with an
//! up/down chevron and opens a menu whose selected row sits over it.

use std::rc::Rc;

use gpui::*;

use crate::icons::{self, Icon};
use crate::theme;

/// Menu geometry, fixed so callers can place and hit-test a menu
/// without measuring: rows are exactly MENU_ROW_H tall and the panel
/// is MENU_W wide with MENU_RADIUS corners. A menu of `rows` is
/// MENU_ROW_H*rows + MENU_PAD*2 tall: 4px of padding and the 1px
/// border at each end.
pub const MENU_W: f32 = 168.0;
pub const MENU_ROW_H: f32 = 24.0;
pub const MENU_RADIUS: f32 = theme::RADIUS_MENU;
pub const MENU_PAD: f32 = 5.0;
/// The leading column of a menu that marks its selected value.
const CHECK_COL: f32 = 18.0;

/// Menu surface, MENU_W wide. Caller fills it with rows.
pub fn menu() -> Div {
    menu_panel().w(px(MENU_W))
}

/// Menu surface sized by the caller (a pop-up button's menu is at least
/// as wide as the button).
fn menu_panel() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(MENU_RADIUS))
        .bg(theme::alpha(theme::BG_ELEV, 0.98))
        .border_1()
        .border_color(theme::HAIRLINE)
        .shadow(theme::shadow_float())
        .py(px(MENU_PAD - 1.0))
        .font_family(theme::FONT)
}

pub fn menu_row(id: &'static str, label: &'static str) -> Stateful<Div> {
    MenuItem::new(id, label).into_row()
}

/// One menu row: a label, optionally a checkmark column, a shortcut
/// column, and the destructive or disabled style. Build with the
/// methods, then `into_row` and attach `on_click`.
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    /// `Some` reserves the checkmark column; `Some(true)` draws the mark.
    checked: Option<bool>,
    shortcut: Option<SharedString>,
    destructive: bool,
    disabled: bool,
}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            checked: None,
            shortcut: None,
            destructive: false,
            disabled: false,
        }
    }

    /// Reserve the checkmark column, marked when `on`. Every row of a
    /// menu that marks a value reserves it, so the labels align.
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    /// The key combination shown right-aligned in tertiary color.
    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    /// Destructive text color (Move to Trash, Discard).
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Disabled: dimmed, with no hover wash. The caller attaches no
    /// click handler to a disabled row.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn into_row(self) -> Stateful<Div> {
        let color = if self.disabled {
            theme::FG_DISABLED
        } else if self.destructive {
            theme::DANGER
        } else {
            theme::FG
        };
        let mut row = div()
            .id(self.id)
            .h(px(MENU_ROW_H))
            .mx(px(MENU_PAD))
            .px(px(if self.checked.is_some() { 4. } else { 9. }))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(4.))
            .text_size(px(theme::TEXT_BODY))
            .whitespace_nowrap()
            .text_color(color)
            // A menu floats above other interactive surfaces; without
            // this the press falls through to whatever is beneath (the
            // toast card's drag tracking, the editor's toolbar buttons).
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        if !self.disabled {
            row = row
                .cursor_pointer()
                .hover(|s| s.bg(theme::ACCENT).text_color(theme::ACCENT_INK));
        }
        if let Some(on) = self.checked {
            row = row.child(
                div()
                    .w(px(CHECK_COL - 4.))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .children(on.then(|| icons::icon(Icon::Check, color, 12.0))),
            );
        }
        row = row.child(div().flex_1().child(self.label));
        if let Some(keys) = self.shortcut {
            row = row.child(div().pl(px(16.)).text_color(theme::FG_FAINT).child(keys));
        }
        row
    }
}

/// A thin separator between groups of menu rows.
pub fn menu_separator() -> Div {
    div()
        .h(px(1.))
        .my(px(4.))
        .mx(px(MENU_PAD + 4.))
        .bg(theme::HAIRLINE)
}

/// Pop-up button: the current value and an up/down chevron on a quiet
/// fill, sized to its content (at least 110 wide), CONTROL_H tall. The
/// caller has the open state and attaches `popup_menu` beside it when
/// open. Toggle it on mouse down from the open state the frame drew: a
/// press on the button while its menu is open runs the menu's
/// press-outside dismissal first, and a toggle that read the state
/// after that dismissal would open the menu again.
pub fn popup_button(
    id: impl Into<ElementId>,
    current: impl Into<SharedString>,
    open: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(theme::CONTROL_H))
        .min_w(px(110.))
        .pl(px(9.))
        .pr(px(5.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(if open {
            theme::SURFACE_PRESS
        } else {
            theme::CONTROL_BG
        })
        .hover(|s| s.bg(theme::alpha(theme::FG, 0.14)))
        .text_size(px(theme::TEXT_BODY))
        .text_color(theme::FG)
        .whitespace_nowrap()
        .cursor_pointer()
        .child(current.into())
        .child(icons::icon(Icon::ChevronUpDown, theme::FG_DIM, 12.0))
}

/// The option menu of a pop-up button, to be added as a child of a
/// `relative` box that holds the button at its top-left. The selected
/// row opens over the button, as on macOS; the menu stays inside the
/// window. Rows mark the selected option. A row click runs
/// `on_select(index)`; a press outside the menu runs `on_dismiss`.
pub fn popup_menu(
    id: &'static str,
    options: &[&'static str],
    selected: Option<usize>,
    min_width: f32,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = Rc::new(on_select);
    let mut panel = menu_panel()
        .id(id)
        .min_w(px(min_width.max(MENU_W - 40.)))
        .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx));
    for (i, opt) in options.iter().enumerate() {
        let on_select = on_select.clone();
        panel = panel.child(
            MenuItem::new(ElementId::NamedInteger(id.into(), i as u64), *opt)
                .checked(selected == Some(i))
                .into_row()
                .on_click(move |_, window, cx| on_select(i, window, cx)),
        );
    }
    deferred(
        anchored()
            .offset(point(px(-(MENU_PAD + 4.)), px(popup_offset(selected))))
            .snap_to_window_with_margin(px(8.))
            .child(panel),
    )
    .with_priority(1)
}

/// Vertical offset of a pop-up menu from its button's top, logical px:
/// the selected row's top lands at the button's top less the row's
/// overhang, so the row's label sits where the button's label was. No
/// selection opens the menu under the button.
pub fn popup_offset(selected: Option<usize>) -> f32 {
    match selected {
        Some(i) => -(MENU_PAD + i as f32 * MENU_ROW_H) + (theme::CONTROL_H - MENU_ROW_H) / 2.0,
        None => theme::CONTROL_H + 4.0,
    }
}
