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

/// Menu surface sized by the caller (a pop-up button's menu is as wide
/// as its widest option, and at least MENU_W - 40).
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
    /// Lit as the pointer's hover lights a row: the highlight that keys
    /// move as well.
    lit: bool,
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
            lit: false,
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

    /// Light the row as the pointer's hover does, for a menu whose
    /// highlight keys move. A key press hides the hover wash until the
    /// pointer moves, so the lit row is the one row lit.
    pub fn lit(mut self, lit: bool) -> Self {
        self.lit = lit;
        self
    }

    pub fn into_row(self) -> Stateful<Div> {
        let lit = self.lit && !self.disabled;
        let color = if self.disabled {
            theme::FG_DISABLED
        } else if lit {
            theme::ACCENT_INK
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
        if lit {
            row = row.bg(theme::ACCENT);
        }
        if let Some(on) = self.checked {
            row = row.child(
                div()
                    .w(px(CHECK_COL - 4.))
                    .flex_shrink_0()
                    .flex()
                    .justify_center()
                    .children(on.then(|| icons::icon_in_text_color(Icon::Check, 12.0))),
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
/// `relative` box that holds the button at its top-left (see
/// [`menu_at`]). The selected row opens over the button, as on macOS;
/// the menu stays inside the window. Rows mark the selected option, and
/// the `lit` row has the highlight, which the caller moves with the
/// pointer and the arrow keys: the pointer entering row `i` runs
/// `on_hover(i, true)`, and leaving it `on_hover(i, false)`. A key press
/// counts as the pointer leaving its row, and its next move as entering
/// it again. A row click runs `on_select(index)`; a press outside the
/// menu runs `on_dismiss`.
pub fn popup_menu(
    id: &'static str,
    options: &[&'static str],
    selected: Option<usize>,
    lit: Option<usize>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
    on_hover: impl Fn(usize, bool, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_select = Rc::new(on_select);
    let on_hover = Rc::new(on_hover);
    let mut panel = menu_panel()
        .id(id)
        .min_w(px(MENU_W - 40.))
        .on_mouse_down_out(move |_, window, cx| on_dismiss(window, cx));
    for (i, opt) in options.iter().enumerate() {
        let (on_select, on_hover) = (on_select.clone(), on_hover.clone());
        panel = panel.child(
            MenuItem::new(ElementId::NamedInteger(id.into(), i as u64), *opt)
                .checked(selected == Some(i))
                .lit(lit == Some(i))
                .into_row()
                .on_hover(move |&entered, window, cx| on_hover(i, entered, window, cx))
                .on_click(move |_, window, cx| on_select(i, window, cx)),
        );
    }
    menu_at(
        point(px(-(MENU_PAD + 4.)), px(popup_offset(selected))),
        panel,
    )
}

/// A floating menu `panel` placed `offset` from the top-left of its
/// `relative` parent, painted over every other element and kept 8 px
/// inside the window. The zero-size absolute box pins the anchor to the
/// parent's top-left: an `anchored` element with no inset sits at its
/// static place in the parent's flow, after the parent's other children.
pub fn menu_at(offset: Point<Pixels>, panel: impl IntoElement) -> Div {
    div().absolute().top_0().left_0().child(
        deferred(
            anchored()
                .offset(offset)
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .with_priority(1),
    )
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

/// What a key does in an open menu (`menu_key`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuKey {
    /// The highlight moved, or stayed at the first or last row.
    Moved,
    /// Choose this row.
    Choose(usize),
    /// Close the menu without a choice.
    Close,
    /// Not a key the menu takes.
    Ignored,
}

/// What `key` does in an open menu of `rows` rows whose highlighted row
/// is `lit`: Up and Down move the highlight to the next row for which
/// `enabled` holds and stop at the first and last such row, Return and
/// Space choose the highlighted row or close a menu with none, and
/// Escape closes the menu. With nothing lit, Down lights the first
/// enabled row and Up the last.
pub fn menu_key(
    lit: &mut Option<usize>,
    rows: usize,
    enabled: impl Fn(usize) -> bool,
    key: &str,
) -> MenuKey {
    let next = match key {
        "down" => (lit.map_or(0, |i| i + 1)..rows).find(|&i| enabled(i)),
        "up" => (0..lit.unwrap_or(rows)).rev().find(|&i| enabled(i)),
        "enter" | "space" => return lit.map_or(MenuKey::Close, MenuKey::Choose),
        "escape" => return MenuKey::Close,
        _ => return MenuKey::Ignored,
    };
    if next.is_some() {
        *lit = next;
    }
    MenuKey::Moved
}

/// Move a menu's highlight `lit` to row `ix` when the pointer enters
/// it, or off `ix` when the pointer leaves it. Returns whether the
/// highlight changed. A disabled row takes no hover, so the pointer on
/// it lights nothing.
pub fn hover_row(lit: &mut Option<usize>, ix: usize, entered: bool) -> bool {
    let next = match (entered, *lit) {
        (true, _) => Some(ix),
        (false, Some(on)) if on == ix => None,
        (false, on) => on,
    };
    std::mem::replace(lit, next) != next
}

#[cfg(test)]
mod tests {
    use super::{hover_row, menu_key, MenuKey};

    // WHY: closes "an open menu takes no keys" and "the pointer and a key
    // each light a row" for every menu: the Settings pop-ups, the
    // editor's Copy and More menus, and the Library's context menu all
    // move their highlight here. Arrows stop at either end and pass over
    // disabled rows, Return and Space choose the lit row, and the
    // pointer moves the same highlight. Not covered: each caller's
    // choice handler, which the X11 popup and menu cases run.
    #[test]
    fn arrows_and_the_pointer_move_one_highlight_over_enabled_rows() {
        let all = |_: usize| true;
        let mut lit = Some(2);
        assert_eq!(menu_key(&mut lit, 5, all, "down"), MenuKey::Moved);
        assert_eq!(lit, Some(3));
        for _ in 0..5 {
            menu_key(&mut lit, 5, all, "down");
        }
        assert_eq!(lit, Some(4), "Down stops at the last row");
        for _ in 0..5 {
            assert_eq!(menu_key(&mut lit, 5, all, "up"), MenuKey::Moved);
        }
        assert_eq!(lit, Some(0), "Up stops at the first row");
        assert_eq!(menu_key(&mut lit, 5, all, "a"), MenuKey::Ignored);
        assert_eq!(lit, Some(0), "another key leaves the highlight");
        assert_eq!(menu_key(&mut lit, 5, all, "enter"), MenuKey::Choose(0));
        assert_eq!(menu_key(&mut lit, 5, all, "space"), MenuKey::Choose(0));
        assert_eq!(menu_key(&mut lit, 5, all, "escape"), MenuKey::Close);

        // With nothing lit, Return closes, Down lights the first row,
        // and Up the last.
        let mut lit = None;
        assert_eq!(menu_key(&mut lit, 5, all, "enter"), MenuKey::Close);
        menu_key(&mut lit, 5, all, "down");
        assert_eq!(lit, Some(0));
        let mut lit = None;
        menu_key(&mut lit, 5, all, "up");
        assert_eq!(lit, Some(4));

        // Disabled rows: 0, 2, and 4 of five. The arrows pass over them,
        // and stop on the last enabled row at either end.
        let odd = |i: usize| i % 2 == 1;
        let mut lit = None;
        menu_key(&mut lit, 5, odd, "down");
        assert_eq!(lit, Some(1), "Down from nothing skips a disabled first row");
        menu_key(&mut lit, 5, odd, "down");
        assert_eq!(lit, Some(3), "Down passes over a disabled row");
        menu_key(&mut lit, 5, odd, "down");
        assert_eq!(lit, Some(3), "Down stops on the last enabled row");
        menu_key(&mut lit, 5, odd, "up");
        menu_key(&mut lit, 5, odd, "up");
        assert_eq!(lit, Some(1), "Up stops on the first enabled row");
        let mut lit = None;
        menu_key(&mut lit, 5, odd, "up");
        assert_eq!(lit, Some(3), "Up from nothing skips a disabled last row");

        // The pointer: entering a row lights it, leaving the lit row
        // clears it, and leaving another row keeps it.
        let mut lit = Some(0);
        assert!(hover_row(&mut lit, 3, true));
        assert_eq!(lit, Some(3));
        assert!(!hover_row(&mut lit, 3, true));
        assert!(!hover_row(&mut lit, 1, false));
        assert_eq!(lit, Some(3));
        assert!(hover_row(&mut lit, 3, false));
        assert_eq!(lit, None);
    }
}
