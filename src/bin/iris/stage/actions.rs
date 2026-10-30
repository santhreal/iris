//! Toast stage actions and UI action helpers.

use std::{
    path::Path,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use super::*;
use crate::{
    icons::Icon,
    motion, pipeline,
    widgets::{tip, MENU_PAD, MENU_ROW_H, MENU_W},
};

impl ToastStage {
    /// Build the stage from an already-scaled thumbnail. The scaling
    /// itself runs on the background executor.
    pub(super) fn from_parts(path: &Path, thumb: Thumb) -> Self {
        Self {
            path: path.to_path_buf(),
            thumb,
            rescaling: false,
            card_screen: (0.0, 0.0, 0.0, 0.0),
            pin_over: None,
            opened: None,
            hover_paused: false,
            pinned: false,
            dismiss_gen: 0,
            closing_at: None,
            exit_from: 0.0,
            drag_start: None,
            gesture: Gesture::Undecided,
            swipe: None,
            swipe_return: None,
            menu_at: None,
            menu_opened: None,
            swallow_click: false,
            pending_morph: None,
            morph_ready_at: None,
            closing_dur: EXIT,
            cfg: iris_lib::config::Config::load(),
            status: None,
            status_at: Instant::now(),
            busy: false,
            hover_turn: (Instant::now(), 0.0),
            cutout: Default::default(),
            hidden: false,
        }
    }

    /// How far the hover actions show now. A screen that does not blend
    /// the window shows them or hides them at once.
    pub(super) fn reveal(&self, still: bool) -> f32 {
        let (turned, from) = self.hover_turn;
        let t = if still {
            1.0
        } else {
            turned.elapsed().as_secs_f32() / motion::tempo(motion::FADE).as_secs_f32()
        };
        hover_reveal(from, self.hover_paused, t)
    }

    /// Start the auto-dismiss countdown. When it fires, hovering pushes
    /// the deadline out instead of dismissing under the pointer.
    pub(super) fn arm_dismiss(&mut self, cx: &mut Context<Self>) {
        if self.pinned || self.closing_at.is_some() {
            return;
        }
        self.dismiss_gen += 1;
        let gen = self.dismiss_gen;
        let sit = Duration::from_millis(u64::from(self.cfg.toast_duration_ms));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(sit).await;
            this.update(cx, |stage, cx| {
                if stage.dismiss_gen != gen || stage.pinned {
                    return;
                }
                if stage.hover_paused || stage.menu_at.is_some() || stage.busy {
                    stage.arm_dismiss(cx);
                } else {
                    stage.begin_close(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn toggle_pin(&mut self, cx: &mut Context<Self>) {
        self.pinned = !self.pinned;
        if self.pinned {
            self.closing_at = None;
        } else {
            self.arm_dismiss(cx);
        }
        cx.notify();
    }

    pub(super) fn begin_close(&mut self, cx: &mut Context<Self>) {
        if self.closing_at.is_some() || self.pending_morph.is_some() {
            return;
        }
        self.closing_at = Some(Instant::now());
        cx.notify();
    }

    /// Replaced by a newer capture: fade out fast from wherever the
    /// card sits. The full fly-off would read as two competing exits.
    pub(super) fn fade_out_quick(&mut self, cx: &mut Context<Self>) {
        self.closing_dur = REPLACE_EXIT;
        self.begin_close(cx);
    }

    /// Open the markup editor over this toast and wait for its first
    /// frame before leaving: the editor paints the same image over the
    /// exact same screen pixels, the capture's rect on the card, so the
    /// morph has no blank blink.
    pub(super) fn hand_off_to_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ready = Arc::new(AtomicBool::new(false));
        let (x, y, _, _) = self.card_screen;
        let from = self.thumb.card.image_at(x, y);
        let result = crate::editor::open(cx, &self.path, Some(from), Some(ready.clone()));
        match result {
            Ok(()) => {
                self.pending_morph = Some((ready, Instant::now()));
                cx.notify();
            }
            Err(e) => {
                iris_lib::ilog!("annotate: {e}");
                crate::widgets::release_render(&self.thumb.render, cx);
                window.remove_window();
                crate::notice::failed(cx, "Editor did not open", &e);
            }
        }
    }

    /// Resolve a released dismiss swipe: fast or far enough flies the
    /// card off from its carried offset; otherwise it springs back to
    /// the corner. No-op once the swipe state is consumed.
    pub(super) fn release_swipe(&mut self, cx: &mut Context<Self>) {
        let Some(mut s) = self.swipe.take() else {
            return;
        };
        // A finger that stopped moving before release is not a flick:
        // decay the smoothed velocity by the sample's age.
        if s.last_at.elapsed() > SWIPE_STALE {
            s.vel = 0.0;
        }
        if s.vel > SWIPE_VELOCITY || s.dx > SWIPE_TRAVEL {
            self.exit_from = s.dx;
            self.begin_close(cx);
        } else {
            self.swipe_return = Some((Instant::now(), s.dx));
        }
        cx.notify();
    }

    /// Show `text` over the card and give it a full dismiss duration
    /// from now, so a result that lands late is still read. The status
    /// holds the slot for STATUS_HOLD; a frame at its end hands the slot
    /// back to the actions under the pointer.
    pub(super) fn show_status(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = Some(text.into());
        self.status_at = Instant::now();
        self.arm_dismiss(cx);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_HOLD).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
        cx.notify();
    }

    /// Run `job` off the UI thread and show its message, or its error,
    /// on the card.
    fn report(
        &mut self,
        job: impl FnOnce() -> Result<String, String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let msg = task.await.unwrap_or_else(|e| e);
            this.update(cx, |stage, cx| {
                stage.busy = false;
                stage.show_status(msg, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn copy_image(&mut self, cx: &mut Context<Self>) {
        // PNG decode of a large capture is too slow for the UI thread.
        let path = self.path.clone();
        self.report(
            move || crate::pipeline::copy_image_file(&path).map(|()| "Copied image".into()),
            cx,
        );
    }

    pub(super) fn copy_file(&mut self, cx: &mut Context<Self>) {
        let path = self.path.clone();
        self.report(
            move || crate::pipeline::copy_file(&path).map(|()| "Copied file".into()),
            cx,
        );
    }

    /// OCR the capture to the clipboard. Recognition takes hundreds of
    /// ms on a large capture; the card holds until it finishes.
    pub(super) fn copy_text(&mut self, cx: &mut Context<Self>) {
        self.busy = true;
        self.show_status("Reading text…", cx);
        let path = self.path.clone();
        self.report(
            move || {
                pipeline::copy_ocr_text(&path)
                    .map(|text| format!("Copied {} characters", text.chars().count()))
            },
            cx,
        );
    }

    pub(super) fn open_folder(&self) {
        crate::sys::reveal::reveal(&self.path);
    }

    pub(super) fn delete_capture(&mut self, cx: &mut Context<Self>) {
        // The trash move + library.json rewrite off the UI thread: a slow
        // shots dir would freeze the toast mid-swipe.
        let path = self.path.clone();
        let task = cx
            .background_executor()
            .spawn(async move { iris_lib::library::trash(&path) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |stage, cx| match result {
                Ok(()) => stage.begin_close(cx),
                Err(e) => stage.show_status(e, cx),
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn perform_click_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.cfg.toast_click_action {
            iris_lib::config::ToastClickAction::Markup => {
                self.hand_off_to_editor(window, cx);
            }
            iris_lib::config::ToastClickAction::Copy => {
                self.copy_image(cx);
            }
            iris_lib::config::ToastClickAction::OpenFolder => {
                self.open_folder();
            }
            iris_lib::config::ToastClickAction::None => {}
        }
    }

    /// Right-click context menu, macOS style: the rows of `ToastRow`.
    /// Fades in over 120ms rising 3px, or shows at once on a screen
    /// that does not blend the window; shares the exit opacity so it
    /// never outlives the card it belongs to. Returns the menu and the
    /// area it paints in the window.
    pub(super) fn render_menu(
        &mut self,
        opacity: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<(Div, crate::sys::window::Rounded)> {
        let (menu_x, menu_y) = self.menu_at?;
        let opened = *self.menu_opened.get_or_insert_with(Instant::now);
        let mt = if crate::sys::window::blended(window) {
            (opened.elapsed().as_secs_f32() / motion::tempo(MENU_FADE).as_secs_f32()).min(1.0)
        } else {
            1.0
        };
        if mt < 1.0 {
            window.request_animation_frame();
        }
        let area = crate::sys::window::Rounded {
            x: menu_x,
            y: menu_y + 3.0 * (1.0 - motion::ease_out_cubic(mt)),
            w: MENU_W,
            h: MENU_H,
            r: crate::widgets::MENU_RADIUS,
        };
        let mut menu = crate::widgets::menu()
            .absolute()
            .left(px(area.x))
            .top(px(area.y))
            .opacity(mt * opacity);
        for row in ToastRow::ALL {
            let mut item = crate::widgets::MenuItem::new(row.id(), row.label());
            if row == ToastRow::Trash {
                item = item.destructive();
            }
            menu = menu.child(item.into_row().on_click(cx.listener(
                move |stage, _, window, cx| {
                    cx.stop_propagation();
                    stage.menu_at = None;
                    stage.run_menu_row(row, window, cx);
                },
            )));
        }
        Some((menu, area))
    }

    fn run_menu_row(&mut self, row: ToastRow, window: &mut Window, cx: &mut Context<Self>) {
        match row {
            ToastRow::Markup => self.hand_off_to_editor(window, cx),
            ToastRow::Copy => self.copy_image(cx),
            ToastRow::CopyText => self.copy_text(cx),
            ToastRow::Pin => {
                crate::pin::open(cx, &self.path, self.pin_over);
                self.begin_close(cx);
            }
            ToastRow::Trash => self.delete_capture(cx),
            ToastRow::Close => self.begin_close(cx),
        }
    }

    /// The hover actions: one frosted capsule centered over the card's
    /// lower edge, in ToastAction::GROUPS split by hairlines. Keep Open
    /// shows when `toast_pin_enabled`. Every button carries a tooltip.
    pub(super) fn render_actions(&self, cx: &mut Context<Self>) -> Option<Div> {
        if !self.cfg.toast_show_actions {
            return None;
        }
        let mut capsule = div()
            .flex()
            .items_center()
            .p(px(CAPSULE_PAD))
            .rounded(px(ACTION_BUTTON / 2.0 + CAPSULE_PAD))
            .bg(theme::alpha(theme::BG_ELEV, 0.96))
            .border_1()
            .border_color(theme::HAIRLINE)
            .shadow(theme::shadow_float());
        let shown =
            |action: &&ToastAction| **action != ToastAction::KeepOpen || self.cfg.toast_pin_enabled;
        let mut groups = ToastAction::GROUPS
            .iter()
            .filter(|group| group.iter().any(|a| shown(&a)))
            .peekable();
        while let Some(group) = groups.next() {
            for &action in group.iter().filter(shown) {
                let (glyph, label) = action.face(self.pinned);
                let active = action == ToastAction::KeepOpen && self.pinned;
                capsule = capsule.child(
                    crate::widgets::hud_icon_button(action.id(), glyph, active, ACTION_BUTTON)
                        .tooltip(tip(label, None))
                        .on_click(cx.listener(move |stage, _, window, cx| {
                            cx.stop_propagation();
                            stage.run_action(action, window, cx);
                        })),
                );
            }
            if groups.peek().is_some() {
                capsule = capsule.child(group_rule());
            }
        }
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(SLOT_INSET))
                .flex()
                .justify_center()
                .child(capsule),
        )
    }

    fn run_action(&mut self, action: ToastAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            ToastAction::KeepOpen => self.toggle_pin(cx),
            ToastAction::CopyImage => self.copy_image(cx),
            ToastAction::CopyFile => self.copy_file(cx),
            ToastAction::Reveal => self.open_folder(),
            ToastAction::Markup => self.hand_off_to_editor(window, cx),
            ToastAction::Trash => self.delete_capture(cx),
        }
    }
}

/// A button of the toast's hover capsule.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ToastAction {
    KeepOpen,
    CopyImage,
    CopyFile,
    Reveal,
    Markup,
    Trash,
}

impl ToastAction {
    /// The capsule's buttons left to right, in groups split by a rule.
    pub(super) const GROUPS: [&'static [ToastAction]; 4] = [
        &[ToastAction::KeepOpen],
        &[ToastAction::CopyImage, ToastAction::CopyFile],
        &[ToastAction::Reveal, ToastAction::Markup],
        &[ToastAction::Trash],
    ];

    fn id(self) -> &'static str {
        match self {
            ToastAction::KeepOpen => "toast-action-pin",
            ToastAction::CopyImage => "toast-action-copy-img",
            ToastAction::CopyFile => "toast-action-copy-file",
            ToastAction::Reveal => "toast-action-open-folder",
            ToastAction::Markup => "toast-action-markup",
            ToastAction::Trash => "toast-action-delete",
        }
    }

    /// The glyph and the tooltip, for a toast `pinned` or not.
    fn face(self, pinned: bool) -> (Icon, &'static str) {
        match self {
            ToastAction::KeepOpen if pinned => (Icon::Pin, "Allow to Close"),
            ToastAction::KeepOpen => (Icon::Pin, "Keep Open"),
            ToastAction::CopyImage => (Icon::Copy, "Copy Image"),
            ToastAction::CopyFile => (Icon::File, "Copy File"),
            ToastAction::Reveal => (Icon::Folder, crate::sys::reveal::LABEL),
            ToastAction::Markup => (Icon::Pen, "Markup"),
            ToastAction::Trash => (Icon::Trash, "Move to Trash"),
        }
    }
}

/// The hover capsule's size with every button shown: the buttons, a
/// rule between groups, the padding, and the 1px border.
pub(super) const CAPSULE: (f32, f32) = {
    let mut buttons = 0;
    let mut g = 0;
    while g < ToastAction::GROUPS.len() {
        buttons += ToastAction::GROUPS[g].len();
        g += 1;
    }
    let rules = ToastAction::GROUPS.len() - 1;
    let rim = 2.0 * (CAPSULE_PAD + 1.0);
    (
        buttons as f32 * ACTION_BUTTON + rules as f32 * (RULE_W + 2.0 * RULE_MARGIN) + rim,
        ACTION_BUTTON + rim,
    )
};

/// A row of the toast's context menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ToastRow {
    Markup,
    Copy,
    CopyText,
    Pin,
    Trash,
    Close,
}

impl ToastRow {
    /// The rows, top to bottom.
    pub(super) const ALL: [ToastRow; 6] = [
        ToastRow::Markup,
        ToastRow::Copy,
        ToastRow::CopyText,
        ToastRow::Pin,
        ToastRow::Trash,
        ToastRow::Close,
    ];

    fn id(self) -> &'static str {
        match self {
            ToastRow::Markup => "toast-markup",
            ToastRow::Copy => "toast-copy",
            ToastRow::CopyText => "toast-ocr",
            ToastRow::Pin => "toast-pin",
            ToastRow::Trash => "toast-delete",
            ToastRow::Close => "toast-close",
        }
    }

    fn label(self) -> &'static str {
        match self {
            ToastRow::Markup => "Markup",
            ToastRow::Copy => "Copy",
            ToastRow::CopyText => "Copy Text",
            ToastRow::Pin => "Pin to Screen",
            ToastRow::Trash => "Move to Trash",
            ToastRow::Close => "Close",
        }
    }
}

/// The context menu's height.
pub(super) const MENU_H: f32 = ToastRow::ALL.len() as f32 * MENU_ROW_H + 2.0 * MENU_PAD;
/// Space kept between the context menu and the window's edge.
pub(super) const MENU_EDGE: f32 = 2.0;

/// Where the context menu opened at window point `at` goes in a
/// `win` (width, height) window: at the pointer, moved left and up to
/// stay MENU_EDGE inside the window.
pub(super) fn menu_place(at: (f32, f32), win: (f32, f32)) -> (f32, f32) {
    (
        at.0.min(win.0 - MENU_W - MENU_EDGE).max(MENU_EDGE),
        at.1.min(win.1 - MENU_H - MENU_EDGE).max(MENU_EDGE),
    )
}

/// Edge of one capsule button: 24, a form control's height; the glyph
/// is 14.
const ACTION_BUTTON: f32 = 24.0;
/// The capsule's padding around its buttons.
const CAPSULE_PAD: f32 = 3.0;
/// The rule between two groups of capsule buttons, and its margin on
/// either side.
const RULE_W: f32 = 1.0;
const RULE_MARGIN: f32 = 3.0;
/// The action slot's inset from the card's lower and side edges.
pub(super) const SLOT_INSET: f32 = 6.0;

/// The hairline between two groups of capsule buttons.
fn group_rule() -> Div {
    div()
        .w(px(RULE_W))
        .h(px(14.))
        .mx(px(RULE_MARGIN))
        .bg(theme::HAIRLINE)
}

/// The pinned toast's mark at rest: an accent disc with the pin glyph in
/// the card's top-right corner. The hover actions show the same state
/// on their pin button.
pub(super) fn pinned_badge() -> Div {
    div()
        .absolute()
        .top(px(SLOT_INSET))
        .right(px(SLOT_INSET))
        .size(px(20.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::ACCENT)
        .shadow(theme::shadow_float())
        .child(crate::icons::icon(Icon::Pin, theme::ACCENT_INK, 12.0))
}

/// The last action's result in the slot along the card's lower edge,
/// on the capsule's frosted fill. The card is the toast's only surface,
/// so a failure shows here, not only in the log. Text wraps to the card
/// width; the card clips the overflow.
pub(super) fn status_band(text: SharedString) -> Div {
    div()
        .absolute()
        .left(px(SLOT_INSET))
        .right(px(SLOT_INSET))
        .bottom(px(SLOT_INSET))
        .flex()
        .justify_center()
        .child(
            div()
                .px(px(10.))
                .py(px(5.))
                .rounded(px(theme::RADIUS_CONTROL + 2.0))
                .bg(theme::alpha(theme::BG_ELEV, 0.96))
                .border_1()
                .border_color(theme::HAIRLINE)
                .shadow(theme::shadow_float())
                .text_size(px(theme::TEXT_SMALL))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::FG)
                .text_center()
                .child(text),
        )
}
