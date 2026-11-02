//! Editor chrome: the toolbar, the tool options bar, the toolbar menus,
//! the status pill, and the shortcuts sheet.

use std::time::Instant;

use gpui::*;

use super::action::{hex_rgba, Transform, COLORS, TOOLS};
use super::Editor;
use crate::icons::Icon;
use crate::theme;
use crate::widgets::{self, tip, ButtonStyle, MenuItem, MenuKey, Segment, Toolbar};

/// The tool options bar's height, under the toolbar.
pub(super) const OPTIONS_H: f32 = 40.0;
/// The stage's top edge: the toolbar and the options bar above it.
pub(super) const STAGE_TOP: f32 = theme::TOOLBAR_H + OPTIONS_H;
/// The clear space between the fit image and each stage edge.
pub(super) const STAGE_MARGIN: f32 = 24.0;

/// The status line's distance from the window's bottom edge when
/// `below` px of stage lie under the image: STATUS_GAP where that room
/// holds the line with STATUS_GAP on each side, else STATUS_GAP above
/// the image's bottom edge, over the image, so the line never lies
/// across that edge.
fn status_bottom(below: f32) -> f32 {
    let gap = widgets::STATUS_GAP;
    if below > 0.0 && below < widgets::STATUS_H + 2.0 * gap {
        below + gap
    } else {
        gap
    }
}

/// The least channel difference, of 1, between a swatch's rim and the
/// options bar under it.
pub(super) const SWATCH_APART: f32 = 16.0 / 255.0;

/// The rim of a swatch on the options bar over its opaque `fill`: the
/// hairline, or where the hairline over the fill would land within
/// SWATCH_APART of the bar, `theme::SWATCH_RIM`. A swatch then shows its
/// whole size: black under the hairline drew at the bar's own gray, a
/// swatch narrower than every other.
pub(super) fn swatch_rim(fill: Rgba) -> Rgba {
    let over = |rim: Rgba| {
        let mix = |r: f32, f: f32| r * rim.a + f * (1.0 - rim.a);
        [mix(rim.r, fill.r), mix(rim.g, fill.g), mix(rim.b, fill.b)]
    };
    let bar = theme::BG_ELEV;
    let apart = over(theme::HAIRLINE)
        .iter()
        .zip([bar.r, bar.g, bar.b])
        .fold(0f32, |m, (c, b)| m.max((c - b).abs()));
    if apart >= SWATCH_APART {
        theme::HAIRLINE
    } else {
        theme::SWATCH_RIM
    }
}

/// The stroke-width stops: glyph, tooltip, and the key that selects it.
const STROKES: [(Icon, &str, &str); 3] = [
    (Icon::Stroke1, "Thin Stroke", "1"),
    (Icon::Stroke2, "Medium Stroke", "2"),
    (Icon::Stroke3, "Thick Stroke", "3"),
];

/// The toolbar's Copy menu: each row's copy variant, label, and the
/// shortcut that runs it, where one does.
const COPY_ROWS: [(&str, &str, Option<&str>); 3] = [
    (
        "image",
        "Copy Image",
        Some(crate::widgets::meta_shortcut!("C")),
    ),
    ("file", "Copy File", None),
    ("path", "Copy Path", None),
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

/// The editor help sheet's tool section: each tool's glyph over the
/// key that selects it, a cap as in the rows under it, read from TOOLS
/// in toolbar order.
fn tool_strip() -> AnyElement {
    div()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(8.))
        .pb(px(10.))
        .mb(px(4.))
        .border_b_1()
        .border_color(theme::HAIRLINE)
        .child(
            div()
                .text_size(px(theme::TEXT_BODY))
                .text_color(theme::FG)
                .child("Tools"),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .gap(px(2.))
                .children(TOOLS.iter().map(|&(_, glyph, _, key)| {
                    div()
                        .w(px(28.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(4.))
                        .child(crate::icons::icon(glyph, theme::FG_DIM, 16.0))
                        .child(crate::widgets::key_hint(key, theme::FG_DIM))
                })),
        )
        .into_any_element()
}

/// The editor help sheet's action and key rows, under the tool strip.
pub(super) fn help_rows() -> Vec<(&'static str, SharedString)> {
    let (save_close, undo_redo) = chord_labels(cfg!(target_os = "macos"));
    vec![
        ("Fill shapes", "F".into()),
        ("Stroke width", "1 / 2 / 3".into()),
        ("Color", "[ / ]".into()),
        ("Zoom fit / in / out", "0 / + / -".into()),
        ("Pan", "Space + drag".into()),
        ("Save and close", save_close.into()),
        ("Discard changes", "Esc twice".into()),
        ("Undo / Redo", undo_redo.into()),
        ("Copy image", crate::widgets::meta_shortcut!("C").into()),
        ("More and Copy menus", "Shift+F10, then Left / Right".into()),
        ("Nudge / delete selection", "Arrows / Delete".into()),
        ("Edit a label", "Double-click".into()),
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
    pub(super) fn open_toolbar_menu(&mut self, copy: bool, open: bool) {
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
        // The Copy and More menus sit side by side on the toolbar, as in
        // a menu bar: either arrow moves to the other one.
        if matches!(key, "left" | "right") {
            self.open_toolbar_menu(!self.copy_menu, true);
            return;
        }
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

    /// The tools as one toolbar segmented control; each tooltip shows
    /// the tool's name and the key that selects it.
    fn tool_buttons(&self, cx: &mut Context<Self>) -> Div {
        let this = cx.entity().downgrade();
        widgets::segmented(
            "editor-tools",
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
        // Undo and Redo each dim, as a disabled toolbar item does, while
        // it has no step to run and no label is open to settle.
        let history = |id: &'static str,
                       glyph: Icon,
                       label: &'static str,
                       keys: &'static str,
                       enabled: bool,
                       run: fn(&mut Self, &mut Context<Self>)| {
            let size = theme::TOOLBAR_CONTROL_H;
            if !enabled {
                return widgets::disabled_icon_button(id, glyph, size)
                    .tooltip(tip(label, Some(keys.into())));
            }
            widgets::icon_button(id, glyph, false, size)
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
            self.can_undo(),
            Self::undo,
        );
        let redo = history(
            "action-redo",
            Icon::Redo,
            "Redo",
            crate::widgets::meta_shortcut!("Shift+Z"),
            self.can_redo(),
            Self::redo,
        );

        // A press toggles from the state this frame drew: with the menu
        // open, its press-outside dismissal runs first.
        let copy_open = self.copy_menu;
        let mut copy = div().relative().child(
            widgets::button_with_icon(
                "btn-copy",
                "Copy",
                Icon::ChevronDown,
                if copy_open {
                    ButtonStyle::Held
                } else {
                    ButtonStyle::Ghost
                },
            )
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
            for (i, (variant, label, keys)) in COPY_ROWS.into_iter().enumerate() {
                let mut item = MenuItem::new(ElementId::Name(variant.into()), label)
                    .lit(self.menu_lit == Some(i));
                if let Some(keys) = keys {
                    item = item.shortcut(keys);
                }
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
            widgets::menu_icon_button(
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

        let done = widgets::button("btn-done", "Done", ButtonStyle::Primary)
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
        widgets::toolbar_bar(window, cx, toolbar, true)
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .opacity(topbar)
            .rounded_t(widgets::window_corner(window, 12.))
            .bg(theme::BG_ELEV)
            .occlude()
    }

    /// The tool options bar: the stroke widths as one segmented control,
    /// the fill toggle, and the markup colors, each set split by a
    /// divider, for the next mark and the selected one. A set the
    /// selected mark, or the tool's next mark, does not take shows
    /// disabled, its value still drawn.
    pub(super) fn render_options(&self, options: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let subject = self
            .selected
            .and_then(|i| self.actions.borrow().get(i).map(|a| a.tool))
            .unwrap_or(self.tool);
        let applies = |takes: bool, el: AnyElement| {
            if takes {
                el
            } else {
                widgets::disabled(el).into_any_element()
            }
        };
        let divider = || div().w(px(1.)).h(px(20.)).mx(px(8.)).bg(theme::HAIRLINE);
        let this = cx.entity().downgrade();
        let strokes = widgets::segmented(
            "stroke",
            STROKES
                .iter()
                .map(|&(glyph, label, key)| Segment::icon(glyph).tip(label, Some(key.into())))
                .collect(),
            Some(usize::from(self.stroke)),
            move |ix, _, cx| {
                this.update(cx, |this, cx| {
                    this.set_stroke(ix as u8);
                    cx.notify();
                })
                .ok();
            },
        );
        let fill_toggle = widgets::icon_button(
            "fill-toggle",
            Icon::Fill,
            self.fill,
            theme::TOOLBAR_CONTROL_H,
        )
        .tooltip(tip("Fill Shapes", Some("F".into())))
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_fill();
            cx.notify();
        }));
        let mut swatches = div().flex().items_center().gap(px(4.));
        for (c, name) in COLORS {
            let active = c == self.color;
            let fill = hex_rgba(c);
            let swatch = div()
                .id(ElementId::Name(c.into()))
                .w(px(24.))
                .h(px(24.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .border_2();
            // The chosen color's ring is the accent; any other swatch
            // rings under the pointer, and in a faded accent while pressed.
            let swatch = if active {
                swatch.border_color(theme::ACCENT)
            } else {
                swatch
                    .border_color(theme::alpha(theme::ACCENT, 0.0))
                    .hover(|s| s.border_color(theme::SWATCH_HOVER))
                    .active(|s| s.border_color(theme::alpha(theme::ACCENT, 0.6)))
            };
            swatches = swatches.child(
                swatch
                    .tooltip(tip(name, None))
                    .child(
                        div()
                            .w(px(16.))
                            .h(px(16.))
                            .rounded_full()
                            .bg(fill)
                            .border_1()
                            .border_color(swatch_rim(fill)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_color(c);
                        cx.notify();
                    })),
            );
        }
        div()
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
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(applies(subject.stroked(), strokes.into_any_element()))
            .child(divider())
            .child(applies(subject.fillable(), fill_toggle.into_any_element()))
            .child(divider())
            .child(applies(subject.colored(), swatches.into_any_element()))
    }

    pub(super) fn render_menus_and_overlays(
        &mut self,
        mut root: Stateful<Div>,
        topbar: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        // Transient status line (copy results, OCR counts, save errors,
        // the discard prompt). It leaves `widgets::STATUS_FOR` after it
        // first shows over the decoded image; a decode failure stays.
        match &self.status {
            None => self.status_since = None,
            Some(text) => {
                let timed = self.status_since.as_ref().is_some_and(|(t, _)| t == text);
                if self.base_ready && !timed {
                    let at = Instant::now();
                    self.status_since = Some((text.clone(), at));
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(widgets::STATUS_FOR).await;
                        let _ = this.update(cx, |this, cx| {
                            if this.status_since.as_ref().is_some_and(|(_, s)| *s == at) {
                                this.status = None;
                                this.status_since = None;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }
                // The discard prompt draws the key it names as a cap.
                let text = widgets::text_with_key(text, "Esc", theme::FG);
                let (_, oy, scale) = self.view(window);
                let below =
                    f32::from(window.bounds().size.height) - oy - self.base_dims.1 as f32 * scale;
                root = root.child(
                    widgets::status_pill(text, STAGE_MARGIN).bottom(px(status_bottom(below))),
                );
            }
        }

        if self.help {
            root = root.child(
                widgets::shortcuts_sheet(Some(tool_strip()), help_rows(), &self.help_scroll)
                    .opacity(topbar)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.help = false;
                        cx.notify();
                    })),
            );
        } else {
            self.help_scroll.set_offset(Point::default());
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
        let keys = |action| rows.iter().find(|(a, _)| *a == action).map(|(_, k)| k);
        assert_eq!(keys("Save and close"), Some(&save_close.into()));
        assert_eq!(keys("Undo / Redo"), Some(&undo_redo.into()));
    }

    // WHY: the class closed here is "a sheet row shows a key as text":
    // each row's keys draw as caps through `widgets::key_hint`, and a
    // row the hint grammar misreads fails here. Not covered: the paint.
    #[test]
    fn every_help_row_reads_as_key_caps() {
        crate::widgets::assert_hints_read_as_keys(&super::help_rows());
    }
}

// WHY: the class closed here is "the status line lies across the
// image's bottom edge", half over the image and half over the stage,
// where the stage under a height-fit image is narrower than the line.
// For every room under the image the line sits clear of the edge: in
// the room with STATUS_GAP on each side, or STATUS_GAP above the edge,
// over the image. Not covered: a line wrapped to more than one row, and
// an image narrower than the line.
#[cfg(test)]
mod status_tests {
    use crate::widgets::{STATUS_GAP, STATUS_H};

    #[test]
    fn the_status_line_never_lies_across_the_image_bottom_edge() {
        for step in -200..=400 {
            let below = step as f32 * 0.25;
            let bottom = super::status_bottom(below);
            let top = bottom + STATUS_H;
            let in_room = bottom >= STATUS_GAP && top + STATUS_GAP <= below;
            let on_image = bottom >= below.max(0.0) + STATUS_GAP;
            assert!(
                in_room || on_image,
                "with {below} px of stage under the image the line spans {bottom}..{top}"
            );
        }
    }

    #[test]
    fn the_status_line_sits_over_the_stage_edge_where_the_room_holds_it() {
        assert_eq!(
            super::status_bottom(STATUS_H + 2.0 * STATUS_GAP),
            STATUS_GAP
        );
        assert_eq!(super::status_bottom(200.0), STATUS_GAP);
        // The image past the window's bottom edge: the line over it.
        assert_eq!(super::status_bottom(-40.0), STATUS_GAP);
    }
}
