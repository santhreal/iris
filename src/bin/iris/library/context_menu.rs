//! The card context menu: a secondary click on a card opens it at the
//! pointer with the actions for the selection.

use gpui::*;

use crate::widgets::{self, menu, menu_separator, MenuItem, MenuKey, MENU_PAD, MENU_ROW_H, MENU_W};

use super::{Library, CARD_W, THUMB_H};
use crate::sys::reveal::LABEL as REVEAL_LABEL;

/// An open context menu: the card it opened on, the window point it
/// opened at, and the row the pointer or the arrow keys light.
pub(super) struct ContextMenu {
    pub index: usize,
    pub at: Point<Pixels>,
    pub lit: Option<usize>,
}

/// A row of the context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MenuRow {
    Open,
    QuickLook,
    Pin,
    Copy,
    Reveal,
    Trash,
}

impl MenuRow {
    /// The rows, top to bottom.
    pub(super) const ALL: [MenuRow; 6] = [
        MenuRow::Open,
        MenuRow::QuickLook,
        MenuRow::Pin,
        MenuRow::Copy,
        MenuRow::Reveal,
        MenuRow::Trash,
    ];

    fn id(self) -> &'static str {
        match self {
            MenuRow::Open => "menu-open",
            MenuRow::QuickLook => "menu-preview",
            MenuRow::Pin => "menu-pin",
            MenuRow::Copy => "menu-copy",
            MenuRow::Reveal => "menu-reveal",
            MenuRow::Trash => "menu-trash",
        }
    }

    fn label(self) -> &'static str {
        match self {
            MenuRow::Open => "Open",
            MenuRow::QuickLook => "Quick Look",
            MenuRow::Pin => "Pin to Screen",
            MenuRow::Copy => "Copy",
            MenuRow::Reveal => REVEAL_LABEL,
            MenuRow::Trash => "Move to Trash",
        }
    }

    fn shortcut(self) -> Option<&'static str> {
        match self {
            MenuRow::Open => Some("Return"),
            MenuRow::QuickLook => Some("Space"),
            MenuRow::Pin => None,
            MenuRow::Copy => Some(crate::widgets::meta_shortcut!("C")),
            MenuRow::Reveal => None,
            MenuRow::Trash => Some("Delete"),
        }
    }

    /// A separator goes above this row.
    const fn starts_group(self) -> bool {
        matches!(self, MenuRow::Copy | MenuRow::Trash)
    }
}

/// The menu's separators.
const SEPARATORS: usize = {
    let (mut n, mut i) = (0, 0);
    while i < MenuRow::ALL.len() {
        if MenuRow::ALL[i].starts_group() {
            n += 1;
        }
        i += 1;
    }
    n
};
/// A separator's height with its margins (`menu_separator`).
const SEPARATOR_H: f32 = 9.0;
/// Space kept between the menu and the window edge.
const EDGE: f32 = 4.0;

/// The menu's height.
pub(super) const MENU_H: f32 =
    MenuRow::ALL.len() as f32 * MENU_ROW_H + SEPARATORS as f32 * SEPARATOR_H + 2.0 * MENU_PAD;

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
        self.menu = Some(ContextMenu {
            index,
            at,
            lit: None,
        });
        cx.notify();
    }

    /// Open the menu from the keyboard (`Shift+F10` or the Menu key) on
    /// the card Return would open, scrolled into view, at its
    /// thumbnail's center.
    pub(super) fn open_menu_at_focus(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.focused() else {
            return;
        };
        self.reveal(index);
        let Some((x, y, ..)) = self.layout.card_rect(index) else {
            return;
        };
        // Document space starts at the scroll viewport's origin and moves
        // up by the scroll amount (the offset is <= 0).
        let view = self.scroll.bounds().origin;
        let at = point(
            view.x + px(x + CARD_W / 2.0),
            view.y + self.scroll.offset().y + px(y + THUMB_H / 2.0),
        );
        self.open_menu(index, at, cx);
    }

    pub(super) fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Run `row` on the menu's card `index` and close the menu.
    fn run_menu_row(
        &mut self,
        row: MenuRow,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_menu(cx);
        match row {
            MenuRow::Open => self.open_entry(index, window, cx),
            MenuRow::QuickLook => self.open_preview(index, cx),
            MenuRow::Pin => {
                if let Some(e) = self.listing.entries().get(index) {
                    // Over the library window, which the pin opens above.
                    let b = window.bounds();
                    let over: crate::pin::Over = (
                        b.origin.x.into(),
                        b.origin.y.into(),
                        b.size.width.into(),
                        b.size.height.into(),
                    );
                    crate::pin::open(cx, &e.path, Some(over));
                }
            }
            MenuRow::Copy => self.copy_selection(cx),
            MenuRow::Reveal => {
                if let Some(e) = self.listing.entries().get(index) {
                    crate::sys::reveal::reveal(&e.path);
                }
            }
            MenuRow::Trash => self.trash_selection(cx),
        }
    }

    /// Run `key` in the open menu. An open menu takes every key: one it
    /// does not use does nothing, and no key acts on the grid under it.
    pub(super) fn menu_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &mut self.menu else {
            return;
        };
        let index = open.index;
        match widgets::menu_key(&mut open.lit, MenuRow::ALL.len(), |_| true, key) {
            MenuKey::Choose(i) => self.run_menu_row(MenuRow::ALL[i], index, window, cx),
            MenuKey::Close => self.close_menu(cx),
            MenuKey::Moved => cx.notify(),
            MenuKey::Ignored => {}
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
        let mut panel = menu().absolute().left(px(x)).top(px(y)).occlude();
        for (i, row) in MenuRow::ALL.into_iter().enumerate() {
            if row.starts_group() {
                panel = panel.child(menu_separator());
            }
            let mut item = MenuItem::new(row.id(), row.label()).lit(open.lit == Some(i));
            if let Some(keys) = row.shortcut() {
                item = item.shortcut(keys);
            }
            if row == MenuRow::Trash {
                item = item.destructive();
            }
            panel = panel.child(
                item.into_row()
                    .on_hover(cx.listener(move |this, entered: &bool, _, cx| {
                        if let Some(open) = &mut this.menu {
                            if widgets::hover_row(&mut open.lit, i, *entered) {
                                cx.notify();
                            }
                        }
                    }))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_menu_row(row, index, window, cx);
                    })),
            );
        }
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
            .child(panel)
    }
}
