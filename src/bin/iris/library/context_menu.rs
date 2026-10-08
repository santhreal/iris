//! The card context menu: a secondary click on a card opens it at the
//! pointer with the actions for the selection.

use gpui::*;

use crate::widgets::{menu, menu_separator, MenuItem, MENU_PAD, MENU_ROW_H, MENU_W};

use super::Library;
use crate::sys::reveal::LABEL as REVEAL_LABEL;

/// An open context menu: the card it opened on and the window point it
/// opened at.
pub(super) struct ContextMenu {
    pub index: usize,
    pub at: Point<Pixels>,
}

/// The menu's rows and separators, top to bottom.
const ROWS: usize = 5;
const SEPARATORS: usize = 2;
/// A separator's height with its margins (`menu_separator`).
const SEPARATOR_H: f32 = 9.0;
/// Space kept between the menu and the window edge.
const EDGE: f32 = 4.0;

/// The menu's height.
pub(super) const MENU_H: f32 =
    ROWS as f32 * MENU_ROW_H + SEPARATORS as f32 * SEPARATOR_H + 2.0 * MENU_PAD;

/// Where a menu of `MENU_W` x `MENU_H` opened at `at` goes in a
/// `win_w` x `win_h` window: at the pointer, moved left to stay inside
/// the window's right edge, and opened upward when it would pass the
/// bottom edge.
pub(super) fn place(at: (f32, f32), win_w: f32, win_h: f32) -> (f32, f32) {
    let x = at.0.min(win_w - MENU_W - EDGE).max(EDGE);
    let y = if at.1 + MENU_H > win_h - EDGE {
        at.1 - MENU_H
    } else {
        at.1
    };
    (x, y.min(win_h - MENU_H - EDGE).max(EDGE))
}

impl Library {
    /// Open the menu on entry `index` at window point `at`. A card
    /// outside the selection becomes the selection, as in a file
    /// manager; a card inside it keeps the selection the menu acts on.
    pub(super) fn open_menu(&mut self, index: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(path) = self.listing.entries().get(index).map(|e| e.path.clone()) else {
            return;
        };
        if !self.selected.contains(&path) {
            self.selected = vec![path];
            self.sel_dirty = true;
        }
        self.cursor = Some(index);
        self.anchor = Some(index);
        self.menu = Some(ContextMenu { index, at });
        cx.notify();
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// The open menu over a full-window dismiss layer: a press outside
    /// the menu closes it.
    pub(super) fn context_menu(
        &self,
        open: &ContextMenu,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let index = open.index;
        let size = window.bounds().size;
        let (x, y) = place(
            (open.at.x.into(), open.at.y.into()),
            size.width.into(),
            size.height.into(),
        );
        let dismiss = cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_menu(cx));
        let dismiss_right = cx.listener(|this, _: &MouseDownEvent, _, cx| this.close_menu(cx));
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .id("menu-dismiss")
                    .absolute()
                    .inset_0()
                    .on_mouse_down(MouseButton::Left, dismiss)
                    .on_mouse_down(MouseButton::Right, dismiss_right),
            )
            .child(
                menu()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .occlude()
                    .child(
                        MenuItem::new("menu-open", "Open")
                            .shortcut("Return")
                            .into_row()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close_menu(cx);
                                this.open_entry(index, window, cx);
                            })),
                    )
                    .child(
                        MenuItem::new("menu-preview", "Quick Look")
                            .shortcut("Space")
                            .into_row()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_menu(cx);
                                this.open_preview(index, cx);
                            })),
                    )
                    .child(menu_separator())
                    .child(
                        MenuItem::new("menu-copy", "Copy")
                            .shortcut("Ctrl+C")
                            .into_row()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_menu(cx);
                                this.copy_selection(cx);
                            })),
                    )
                    .child(
                        MenuItem::new("menu-reveal", REVEAL_LABEL)
                            .into_row()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_menu(cx);
                                if let Some(e) = this.listing.entries().get(index) {
                                    crate::sys::reveal::reveal(&e.path);
                                }
                            })),
                    )
                    .child(menu_separator())
                    .child(
                        MenuItem::new("menu-trash", "Move to Trash")
                            .shortcut("Delete")
                            .destructive()
                            .into_row()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_menu(cx);
                                this.trash_selection(cx);
                            })),
                    ),
            )
    }
}
