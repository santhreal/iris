//! Editor chrome: the toolbar, the tool options bar, the toolbar menus,
//! the status pill, and the shortcuts sheet.

use gpui::*;

use super::action::{hex_rgba, Transform, COLORS, TOOLS};
use super::Editor;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, tip, MenuItem, MenuKey, Segment, SegmentStyle, Toolbar};

/// The tool options bar's height, under the toolbar.
pub(super) const OPTIONS_H: f32 = 40.0;
/// The stage's top edge: the toolbar and the options bar above it.
pub(super) const STAGE_TOP: f32 = theme::TOOLBAR_H + OPTIONS_H;
/// The clear space between the fit image and each stage edge.
pub(super) const STAGE_MARGIN: f32 = 24.0;

/// The stroke-width stops: glyph, tooltip, and the key that selects it.
const STROKES: [(Icon, &str, &str); 3] = [
    (Icon::Stroke1, "Thin Stroke", "1"),
    (Icon::Stroke2, "Medium Stroke", "2"),
    (Icon::Stroke3, "Thick Stroke", "3"),
];

/// The toolbar's Copy menu: each row's copy variant and label.
const COPY_ROWS: [(&str, &str); 3] = [
    ("image", "Copy Image"),
    ("file", "Copy File"),
    ("path", "Copy Path"),
];

/// A row of the toolbar's More menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum More {
    CopyText,
    Rotate,
    FlipHorizontal,
    FlipVertical,
    ClearMarkup,
    Shortcuts,
    Discard,
}

impl More {
    pub(super) const ALL: [More; 7] = [
        More::CopyText,
        More::Rotate,
        More::FlipHorizontal,
        More::FlipVertical,
        More::ClearMarkup,
        More::Shortcuts,
        More::Discard,
    ];

    fn label(self) -> &'static str {
        match self {
            More::CopyText => "Copy Text",
            More::Rotate => "Rotate Right",
            More::FlipHorizontal => "Flip Horizontal",
            More::FlipVertical => "Flip Vertical",
            More::ClearMarkup => "Clear Markup",
            More::Shortcuts => "Keyboard Shortcuts",
            More::Discard => "Discard",
        }
    }

    fn shortcut(self) -> Option<&'static str> {
        match self {
            More::Shortcuts => Some("?"),
            More::Discard => Some("Esc"),
            _ => None,
        }
    }

    /// A separator goes above this row.
    fn starts_group(self) -> bool {
        matches!(self, More::ClearMarkup | More::Shortcuts | More::Discard)
    }
}

/// A toolbar menu panel dropped 4 px under its button, inside the
/// window.
fn drop_menu(panel: impl IntoElement) -> impl IntoElement {
    widgets::menu_at(point(px(0.), px(theme::TOOLBAR_CONTROL_H + 4.)), panel)
}

fn chord_labels(macos: bool) -> (&'static str, &'static str) {
    if macos {
        ("Cmd+S / Cmd+W / Enter", "Cmd+Z / Cmd+Shift+Z")
    } else {
        ("Ctrl+S / Ctrl+W / Enter", "Ctrl+Z / Ctrl+Shift+Z")
    }
}

/// The editor help sheet's action and key rows.
pub(super) fn help_rows() -> Vec<(&'static str, SharedString)> {
    let (save_close, undo_redo) = chord_labels(cfg!(target_os = "macos"));
    vec![
        ("Tools", "V P L A E R T H B C N".into()),
        ("Fill shapes", "F".into()),
        ("Stroke width", "1 / 2 / 3".into()),
        ("Zoom fit / in / out", "0 / + / -".into()),
        ("Pan", "Space + drag".into()),
        ("Save and close", save_close.into()),
        ("Discard changes", "Esc twice".into()),
        ("Undo / Redo", undo_redo.into()),
        ("Delete selected action", "Delete".into()),
        ("Apply crop", "Enter".into()),
        ("This sheet", "?".into()),
    ]
}

impl Editor {
    /// Run a More menu row: the same operation its former toolbar
    /// button ran.
    pub(super) fn run_more(&mut self, item: More, window: &mut Window, cx: &mut Context<Self>) {
        self.more_menu = false;
        match item {
            More::CopyText => self.copy_text_ocr(cx),
            More::Rotate => self.transform(Transform::Rot90, cx),
            More::FlipHorizontal => self.transform(Transform::FlipH, cx),
            More::FlipVertical => self.transform(Transform::FlipV, cx),
            More::ClearMarkup => self.clear(),
            More::Shortcuts => self.help = true,
            More::Discard => self.discard(window, cx),
        }
        cx.notify();
    }

    /// Whether More menu row `item` runs: Clear Markup has nothing to
    /// clear on an unmarked image.
    fn more_enabled(&self, item: More) -> bool {
        item != More::ClearMarkup || !self.actions.borrow().is_empty()
    }

    /// Open the Copy menu (`copy`) or the More menu with nothing lit,
    /// closing the other; `open` false closes both.
    fn open_toolbar_menu(&mut self, copy: bool, open: bool) {
        self.copy_menu = open && copy;
        self.more_menu = open && !copy;
        self.menu_lit = None;
    }

    /// Run `key` in the open Copy or More menu. An open menu takes every
    /// key: one it does not use does nothing, and no key reaches the
    /// image or the toolbar under it.
    pub(super) fn toolbar_menu_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.copy_menu {
            match widgets::menu_key(&mut self.menu_lit, COPY_ROWS.len(), |_| true, key) {
                MenuKey::Choose(i) => self.copy_variant(COPY_ROWS[i].0, cx),
                MenuKey::Close => self.copy_menu = false,
                MenuKey::Moved | MenuKey::Ignored => {}
            }
            return;
        }
        let enabled = More::ALL.map(|item| self.more_enabled(item));
        match widgets::menu_key(&mut self.menu_lit, More::ALL.len(), |i| enabled[i], key) {
            MenuKey::Choose(i) => self.run_more(More::ALL[i], window, cx),
            MenuKey::Close => self.more_menu = false,
            MenuKey::Moved | MenuKey::Ignored => {}
        }
    }

    /// A Copy or More menu row's hover listener: the pointer entering
    /// row `ix` lights it, and leaving it clears the highlight.
    fn menu_row_hover(
        this: WeakEntity<Self>,
        ix: usize,
    ) -> impl Fn(&bool, &mut Window, &mut App) + 'static {
        move |entered, _, cx| {
            this.update(cx, |this, cx| {
                if widgets::hover_row(&mut this.menu_lit, ix, *entered) {
                    cx.notify();
                }
            })
            .ok();
        }
    }

    pub(super) fn render_backdrop(&self, chrome: f32, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .left_0()
            .top(px(STAGE_TOP))
            .right_0()
            .bottom_0()
            .id("backdrop")
            .opacity(chrome)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _ev: &MouseDownEvent, _, _| {
                    // A press outside the image settles an open text
                    // entry and keeps it; Discard is explicit.
                    this.commit_text(true);
                }),
            )
    }

    /// The tools as one Toolbar segmented control; each tooltip shows
    /// the tool's name and the key that selects it.
    fn tool_buttons(&self, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        widgets::segmented(
            "editor-tools",
            SegmentStyle::Toolbar,
            TOOLS
                .iter()
                .map(|&(_, glyph, label, key)| Segment::icon(glyph).tip(label, Some(key.into())))
                .collect(),
            TOOLS.iter().position(|t| t.0 == self.tool),
            move |ix, _, cx| {
                this.update(cx, |this, cx| this.set_tool(TOOLS[ix].0, cx))
                    .ok();
            },
        )
    }

    pub(super) fn render_topbar(
        &self,
        topbar: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let history = |id: &'static str,
                       glyph: Icon,
                       label: &'static str,
                       keys: &'static str,
                       run: fn(&mut Self, &mut Context<Self>)| {
            widgets::icon_button(id, glyph, false, theme::TOOLBAR_CONTROL_H)
                .tooltip(tip(label, Some(keys.into())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    run(this, cx);
                    cx.notify();
                }))
        };
        let undo = history(
            "action-undo",
            Icon::Undo,
            "Undo",
            crate::widgets::meta_shortcut!("Z"),
            Self::undo,
        );
        let redo = history(
            "action-redo",
            Icon::Redo,
            "Redo",
            crate::widgets::meta_shortcut!("Shift+Z"),
            Self::redo,
        );

        // A press toggles from the state this frame drew: with the menu
        // open, its press-outside dismissal runs first.
        let copy_open = self.copy_menu;
        let mut copy = div().relative().child(
            widgets::button_with_icon("btn-copy", "Copy", Icon::ChevronDown, false)
                .tooltip(tip("Copy Image, File, or Path", None))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.open_toolbar_menu(true, !copy_open);
                        cx.notify();
                    }),
                ),
        );
        if copy_open {
            let mut panel = widgets::menu()
                .id("copy-menu")
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.copy_menu = false;
                    cx.notify();
                }));
            for (i, (variant, label)) in COPY_ROWS.into_iter().enumerate() {
                let item = MenuItem::new(ElementId::Name(variant.into()), label)
                    .lit(self.menu_lit == Some(i));
                panel = panel.child(
                    item.into_row()
                        .on_hover(Self::menu_row_hover(cx.entity().downgrade(), i))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_variant(variant, cx);
                        })),
                );
            }
            copy = copy.child(drop_menu(panel));
        }

        let more_open = self.more_menu;
        let mut more = div().relative().child(
            widgets::icon_button(
                "btn-more",
                Icon::Ellipsis,
                more_open,
                theme::TOOLBAR_CONTROL_H,
            )
            .tooltip(tip("More", None))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.open_toolbar_menu(false, !more_open);
                    cx.notify();
                }),
            ),
        );
        if more_open {
            let mut panel = widgets::menu()
                .id("more-menu")
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.more_menu = false;
                    cx.notify();
                }));
            for (i, item) in More::ALL.into_iter().enumerate() {
                if item.starts_group() {
                    panel = panel.child(widgets::menu_separator());
                }
                let enabled = self.more_enabled(item);
                let mut row = MenuItem::new(ElementId::Name(item.label().into()), item.label())
                    .disabled(!enabled)
                    .lit(self.menu_lit == Some(i));
                if let Some(keys) = item.shortcut() {
                    row = row.shortcut(keys);
                }
                if item == More::Discard {
                    row = row.destructive();
                }
                let mut row = row.into_row();
                if enabled {
                    row = row
                        .on_hover(Self::menu_row_hover(cx.entity().downgrade(), i))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.run_more(item, window, cx)),
                        );
                }
                panel = panel.child(row);
            }
            more = more.child(drop_menu(panel));
        }

        let done = widgets::button("btn-done", "Done", true)
            .tooltip(tip("Save and Close", Some("Enter".into())))
            .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx)));

        let toolbar = Toolbar::new(self.filename.clone())
            .caption(self.dims.clone())
            .center(self.tool_buttons(cx))
            .trailing(undo)
            .trailing(redo)
            .trailing(copy)
            .trailing(more)
            .trailing(done)
            .on_close(self.on_close.clone());
        // The toolbar is opaque and over the stage: a zoomed image runs
        // under it and never takes its presses, hover, or wheel.
        widgets::toolbar_bar(window, toolbar, true)
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .opacity(topbar)
            .rounded_t(widgets::window_corner(window, 12.))
            .bg(theme::BG_ELEV)
            .occlude()
    }

    /// The tool options bar: stroke width, the fill toggle, and the
    /// markup colors for new actions.
    pub(super) fn render_options(&self, options: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let mut bar = div()
            .id("tool-options")
            .absolute()
            .top(px(theme::TOOLBAR_H))
            .left_0()
            .right_0()
            .h(px(OPTIONS_H))
            .opacity(options)
            .flex()
            .items_center()
            .justify_center()
            .gap(px(4.))
            .bg(theme::BG_ELEV)
            .border_t_1()
            .border_b_1()
            .border_color(theme::HAIRLINE)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, (glyph, label, key)) in STROKES.into_iter().enumerate() {
            bar = bar.child(
                widgets::icon_button(
                    ElementId::NamedInteger("stroke".into(), i as u64),
                    glyph,
                    self.stroke == i as u8,
                    theme::TOOLBAR_CONTROL_H,
                )
                .tooltip(tip(label, Some(key.into())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.stroke = i as u8;
                    cx.notify();
                })),
            );
        }
        bar = bar.child(
            widgets::icon_button(
                "fill-toggle",
                Icon::Fill,
                self.fill,
                theme::TOOLBAR_CONTROL_H,
            )
            .tooltip(tip("Fill Shapes", Some("F".into())))
            .on_click(cx.listener(|this, _, _, cx| {
                this.fill = !this.fill;
                cx.notify();
            })),
        );
        bar = bar.child(div().w(px(1.)).h(px(20.)).mx(px(8.)).bg(theme::HAIRLINE));
        for c in COLORS {
            let active = c == self.color;
            bar = bar.child(
                div()
                    .id(ElementId::Name(c.into()))
                    .w(px(24.))
                    .h(px(24.))
                    .flex_shrink_0()
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_2()
                    .border_color(if active {
                        theme::ACCENT
                    } else {
                        theme::alpha(theme::ACCENT, 0.0)
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded_full()
                            .bg(hex_rgba(c))
                            .border_1()
                            .border_color(theme::HAIRLINE),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.color = c;
                        cx.notify();
                    })),
            );
        }
        bar
    }

    pub(super) fn render_menus_and_overlays(
        &mut self,
        mut root: Stateful<Div>,
        topbar: f32,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        // Transient status line (copy results, OCR counts, save errors).
        if let Some(status) = &self.status {
            root = root.child(widgets::status_pill(status, STAGE_MARGIN));
        }

        if self.help {
            root = root.child(
                widgets::shortcuts_sheet(help_rows())
                    .opacity(topbar)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.help = false;
                        cx.notify();
                    })),
            );
        }

        root
    }
}

#[cfg(test)]
mod shortcut_tests {
    // WHY: editor history, save, and close accept the platform modifier.
    // Their labels must present Command on macOS and Control elsewhere;
    // both target mappings run on every host.
    #[test]
    fn help_rows_follow_each_platform_convention() {
        assert_eq!(
            [super::chord_labels(false), super::chord_labels(true)],
            [
                ("Ctrl+S / Ctrl+W / Enter", "Ctrl+Z / Ctrl+Shift+Z"),
                ("Cmd+S / Cmd+W / Enter", "Cmd+Z / Cmd+Shift+Z"),
            ]
        );
        let rows = super::help_rows();
        let (save_close, undo_redo) = super::chord_labels(cfg!(target_os = "macos"));
        assert_eq!(rows[5], ("Save and close", save_close.into()));
        assert_eq!(rows[7], ("Undo / Redo", undo_redo.into()));
    }
}
