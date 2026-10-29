//! Notice: the corner card for an outcome no other surface shows. A
//! recording that saved, and a capture, recording, or command that
//! failed after its trigger (a hotkey, the tray, the chip, a forwarded
//! command line) returned.
//!
//! The card lands in the toast's corner of the primary display, beyond
//! a live toast. It slides in from past the screen edge, stays for the
//! toast duration (a failure for at least 8 seconds; hovering holds
//! it), and slides back out. A newer notice replaces it, and so does a
//! toast that lands on it. A saved file offers Show in Folder. The
//! window never takes focus.
//!
//! A guide is the one notice that does not time out: it states the
//! next step of an interaction that is waiting on input (click a
//! window to record it) and leaves when that interaction ends.
//!
//! A countdown is the card of a delayed capture. It counts the seconds
//! down, and its close button cancels the capture. At zero the card
//! closes, and the capture reads the screen once it is gone.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::*;

use crate::icons::{icon, Icon};
use crate::widgets::{self, icon_button};
use crate::{daemon, motion, theme};

#[cfg(test)]
mod tests;

/// A screen rect: x, y, width, height in logical px.
pub type Rect = (f32, f32, f32, f32);

/// Card width, logical px.
const W: f32 = 380.0;
/// Gap between the card and the screen edge, as for the toast.
const MARGIN: f32 = 12.0;
/// Room on the card's inner sides for its shadow.
const BLEED: f32 = theme::CARD_BLEED;
/// Gap between the card and a live toast card it stacks beyond: the
/// shadow room on either card's inner side, so neither window reaches
/// into the other card and neither cuts its shadow short of it.
const STACK_GAP: f32 = BLEED;
const PAD: f32 = 12.0;
/// The card's hairline, inside its width and height.
const BORDER: f32 = 1.0;
const GLYPH: f32 = 18.0;
/// A form control's edge: 24, with a 14px glyph.
const BUTTON: f32 = 24.0;
const BUTTON_GAP: f32 = 4.0;
const GAP: f32 = 10.0;
/// The title's line is the button row's height, so the 13px headline
/// and the glyph center on the buttons beside them.
const TITLE_LINE: f32 = BUTTON;
const TITLE_GAP: f32 = 0.0;
const DETAIL_LINE: f32 = 15.0;
/// A longer detail ends in an ellipsis on its last line.
const MAX_LINES: usize = 3;
/// A failure stays at least this long: it is read, not glanced at.
const FAILURE_HOLD: Duration = Duration::from_secs(8);
const EXIT: Duration = Duration::from_millis(300);
/// Replaced by a newer notice or a toast: a fast fade, not the slide.
const REPLACE_EXIT: Duration = Duration::from_millis(180);

/// The live notice, so a newer one replaces it.
static NOTICE: parking_lot::Mutex<Option<WindowHandle<Notice>>> = parking_lot::Mutex::new(None);
/// The delayed capture that runs: a newer one or a cancel moves it on,
/// and a countdown whose number has moved on does not capture.
static COUNTDOWN: AtomicU64 = AtomicU64::new(0);

/// What a card reports, which sets its glyph, its buttons, and how
/// long it stays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Saved,
    Failure,
    Update,
    Guide,
    /// A delayed capture that runs at the deadline.
    Countdown(Instant),
}

/// A file was written: its name, and Show in Folder.
pub fn saved(cx: &mut App, title: &str, path: &Path) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    show(cx, title, name, Some(path.to_path_buf()), Kind::Saved);
}

/// `what` failed with `err`.
pub fn failed(cx: &mut App, what: &str, err: &str) {
    show(cx, what, one_paragraph(err), None, Kind::Failure);
}

/// The next step of an interaction waiting on input. The card stays
/// until `end_guide` or a newer notice replaces it.
#[cfg(target_os = "linux")]
pub fn guide(cx: &mut App, title: &str, detail: &str) {
    show(cx, title, detail.to_string(), None, Kind::Guide);
}

/// The interaction a guide described ended: a live guide leaves. Any
/// other live notice stays.
#[cfg(target_os = "linux")]
pub fn end_guide(cx: &mut App) {
    let live = *NOTICE.lock();
    if let Some(handle) = live {
        let _ = handle.update(cx, |n, _, cx| {
            if n.kind == Kind::Guide {
                n.leave(REPLACE_EXIT, cx);
            }
        });
    }
}

/// Capture every display in `secs` seconds, counting down on a card
/// whose close button cancels the capture. A newer countdown replaces
/// this one, card and capture. A newer notice that replaces the card
/// leaves the capture to run. Zero seconds captures at once.
pub fn countdown(cx: &mut App, secs: u64) {
    let run = COUNTDOWN.fetch_add(1, Ordering::Relaxed) + 1;
    if secs == 0 {
        capture_now(cx);
        return;
    }
    let deadline = Instant::now() + Duration::from_secs(secs);
    show(cx, "", String::new(), None, Kind::Countdown(deadline));
    cx.spawn(async move |cx| {
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            // Wake when the whole second the card shows changes.
            let part = left - Duration::from_secs(left.as_secs());
            let wait = if part.is_zero() {
                Duration::from_secs(1)
            } else {
                part
            };
            cx.background_executor().timer(wait).await;
            cx.update(|cx| on_countdown(cx, |_, _, cx| cx.notify()));
        }
        cx.update(|cx| {
            if COUNTDOWN.load(Ordering::Relaxed) == run {
                capture_now(cx);
            }
        });
    })
    .detach();
}

/// Run `f` on the live notice when it is a countdown on screen. False
/// when there is none.
fn on_countdown(
    cx: &mut App,
    f: impl FnOnce(&mut Notice, &mut Window, &mut Context<Notice>),
) -> bool {
    let Some(handle) = *NOTICE.lock() else {
        return false;
    };
    handle
        .update(cx, |n, window, cx| {
            let live = matches!(n.kind, Kind::Countdown(_)) && n.closing.is_none();
            if live {
                f(n, window, cx);
            }
            live
        })
        .unwrap_or(false)
}

/// The countdown reached zero: its card closes, then the capture runs
/// on a screen without it.
fn capture_now(cx: &mut App) {
    let closed = on_countdown(cx, |n, window, cx| {
        if crate::sys::window::blended(window) {
            n.hidden = true;
            cx.notify();
            widgets::close_then_run(window, cx, daemon::Command::CaptureFullscreen);
        } else {
            // A card cut to its shape leaves the screen as it closes.
            window.remove_window();
            widgets::run_after_vanish(cx, daemon::Command::CaptureFullscreen);
        }
    });
    if !closed {
        daemon::run(cx, &daemon::Command::CaptureFullscreen);
    }
}

/// The title of a countdown with `left` to go: the whole seconds,
/// rounded up, so the card reads 1 in the last second.
fn countdown_title(left: Duration) -> String {
    match left.as_nanos().div_ceil(1_000_000_000).max(1) {
        1 => "Screenshot in 1 second".to_string(),
        n => format!("Screenshot in {n} seconds"),
    }
}

/// iris `version` is available: Install hands off the offered update.
pub fn update_available(cx: &mut App, version: &semver::Version) {
    show(
        cx,
        &format!("iris {version} is available"),
        "Install it now, or later from the tray menu or Settings.".to_string(),
        None,
        Kind::Update,
    );
}

/// A toast card landed on `toast`: a notice under it leaves, as for a
/// newer notice.
pub fn yield_to(cx: &mut App, toast: Rect) {
    let live = *NOTICE.lock();
    if let Some(handle) = live {
        let _ = handle.update(cx, |n, _, cx| {
            if overlaps(n.card, toast) {
                n.leave(REPLACE_EXIT, cx);
            }
        });
    }
}

/// `err` as one paragraph: line breaks and runs of whitespace (ffmpeg's
/// stderr) collapse to single spaces, so the wrap estimate holds.
fn one_paragraph(err: &str) -> String {
    err.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn show(cx: &mut App, title: &str, detail: String, file: Option<PathBuf>, kind: Kind) {
    if let Err(e) = open(cx, title, detail, file, kind) {
        iris_lib::ilog!("iris: notice: {e}");
    }
}

pub struct Notice {
    title: SharedString,
    detail: SharedString,
    /// Detail lines the card is sized for; 0 without a detail.
    lines: usize,
    file: Option<PathBuf>,
    kind: Kind,
    height: f32,
    /// The card's rect on screen, for a landing toast's overlap test.
    card: Rect,
    /// The card's distance below its window's top edge.
    inset: f32,
    left: bool,
    hold: Duration,
    opened: Option<Instant>,
    closing: Option<(Instant, Duration)>,
    hovered: bool,
    /// Dismiss-arm generation: a timer that wakes on an older one is
    /// a no-op, so the last hover-out's deadline is the one that holds.
    armed: u64,
    /// The card, where a screen that does not blend the window shows it.
    cutout: crate::sys::window::Cutout,
    /// Draw nothing: a countdown at zero leaves the screen to the capture.
    hidden: bool,
}

/// Width of the detail text beside the glyph and `buttons` buttons.
fn text_width(buttons: usize) -> f32 {
    let row = match buttons {
        0 => 0.0,
        n => n as f32 * BUTTON + (n - 1) as f32 * BUTTON_GAP + GAP,
    };
    W - 2.0 * (BORDER + PAD) - GLYPH - GAP - row
}

/// The buttons on a card of `kind`: reveal with a file, install on an
/// update, and close on all but a guide. A guide's pick grabs the
/// pointer, so a button on it could never be pressed; Escape ends it.
fn buttons(kind: Kind, file: bool) -> usize {
    usize::from(file) + usize::from(kind == Kind::Update) + usize::from(kind != Kind::Guide)
}

/// Card height for `lines` detail lines.
fn card_height(lines: usize) -> f32 {
    let text = if lines == 0 {
        TITLE_LINE
    } else {
        TITLE_LINE + TITLE_GAP + lines as f32 * DETAIL_LINE
    };
    2.0 * (BORDER + PAD) + text.max(BUTTON)
}

/// True when `a` and `b` share any area; touching edges do not.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
}

/// Window and card rects for a card of `height` in the given corner of
/// `screen`. The window holds BLEED beside the card for its shadow and
/// MARGIN on a side at the screen edge. A live `toast` card the corner
/// spot would cover keeps its place: the card moves STACK_GAP past it,
/// away from the edge, and the window then holds BLEED on that side
/// too, ending where the toast card begins.
fn place(screen: Rect, left: bool, top: bool, height: f32, toast: Option<Rect>) -> (Rect, Rect) {
    let (sx, sy, sw, sh) = screen;
    let win_w = W + BLEED + MARGIN;
    let ox = if left { sx } else { sx + sw - win_w };
    let x = if left { ox + MARGIN } else { ox + BLEED };
    let y = if top {
        sy + MARGIN
    } else {
        sy + sh - MARGIN - height
    };
    match toast.filter(|&t| overlaps((x, y, W, height), t)) {
        None => {
            let oy = if top { sy } else { y - BLEED };
            ((ox, oy, win_w, height + BLEED + MARGIN), (x, y, W, height))
        }
        Some(t) => {
            let y = if top {
                t.1 + t.3 + STACK_GAP
            } else {
                t.1 - STACK_GAP - height
            };
            (
                (ox, y - BLEED, win_w, height + 2.0 * BLEED),
                (x, y, W, height),
            )
        }
    }
}

fn open(
    cx: &mut App,
    title: &str,
    detail: String,
    file: Option<PathBuf>,
    kind: Kind,
) -> Result<(), String> {
    use iris_lib::config::ToastPosition;
    let cfg = iris_lib::config::Config::load();
    let left = matches!(
        cfg.toast_position,
        ToastPosition::BottomLeft | ToastPosition::TopLeft
    );
    let top = matches!(
        cfg.toast_position,
        ToastPosition::TopLeft | ToastPosition::TopRight
    );
    let hold = Duration::from_millis(u64::from(cfg.toast_duration_ms));
    let hold = match kind {
        Kind::Failure | Kind::Update => hold.max(FAILURE_HOLD),
        Kind::Saved | Kind::Guide | Kind::Countdown(_) => hold,
    };
    let buttons = buttons(kind, file.is_some());
    // The detail wraps as GPUI's text element wraps it, measured before
    // layout, so the card height is known before the window opens.
    let lines = if detail.trim().is_empty() {
        0
    } else {
        theme::wrapped_lines(
            cx,
            &detail,
            theme::TEXT_SMALL,
            FontWeight::NORMAL,
            text_width(buttons),
        )
        .min(MAX_LINES)
    };
    let height = card_height(lines);
    let screen = crate::sys::window::primary_monitor_rect(cx).ok_or("no display")?;
    let toast = crate::stage::live_card_rect(cx);
    let (win, card) = place(screen, left, top, height, toast);

    let notice = Notice {
        title: SharedString::from(title.to_string()),
        detail: SharedString::from(detail),
        lines,
        file,
        kind,
        height,
        card,
        inset: card.1 - win.1,
        left,
        hold,
        opened: None,
        closing: None,
        hovered: false,
        armed: 0,
        cutout: Default::default(),
        hidden: false,
    };
    let handle = crate::widgets::open_window(
        cx,
        "Notice - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(win.0), px(win.1)),
                size: size(px(win.2), px(win.3)),
            })),
            titlebar: None,
            focus: false,
            show: true,
            kind: WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            display_id: None,
            window_background: WindowBackgroundAppearance::Transparent,
            window_min_size: None,
            window_decorations: Some(WindowDecorations::Client),
            tabbing_identifier: None,
            ..Default::default()
        },
        |_, cx| cx.new(|_| notice),
    )
    .map_err(|e| format!("open notice window: {e}"))?;
    let previous = NOTICE.lock().replace(handle);
    if let Some(previous) = previous {
        let _ = previous.update(cx, |n, _, cx| n.leave(REPLACE_EXIT, cx));
    }
    if !matches!(kind, Kind::Guide | Kind::Countdown(_)) {
        handle
            .update(cx, |n, _, cx| {
                let wait = motion::tempo(motion::ENTER) + n.hold;
                n.arm(wait, cx);
            })
            .map_err(|e| format!("arm notice dismiss: {e}"))?;
    }
    Ok(())
}

impl Notice {
    /// Leave after `wait`, or later while the pointer rests on the card.
    /// A guide and a countdown stay until their interaction ends.
    fn arm(&mut self, wait: Duration, cx: &mut Context<Self>) {
        if self.closing.is_some() || matches!(self.kind, Kind::Guide | Kind::Countdown(_)) {
            return;
        }
        self.armed += 1;
        let armed = self.armed;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |n, cx| {
                if n.armed != armed {
                    return;
                }
                if n.hovered {
                    let hold = n.hold;
                    n.arm(hold, cx);
                } else {
                    n.leave(EXIT, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn leave(&mut self, over: Duration, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some((Instant::now(), over));
            cx.notify();
        }
    }
}

impl Render for Notice {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.hidden {
            return div();
        }
        // A screen that does not blend the window shows the card still,
        // in its cut-out area: it arrives and leaves at once.
        let still = !crate::sys::window::blended(window);
        // Entrance: decelerating hard into the corner, as the toast.
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let enter_t = if still {
            1.0
        } else {
            (opened.elapsed().as_secs_f32() / motion::tempo(motion::ENTER).as_secs_f32()).min(1.0)
        };
        let ease = 1.0 - (1.0 - enter_t).powi(4);
        let mut offset = MARGIN - (1.0 - ease) * (W + 2.0 * MARGIN);
        let mut opacity = 1.0f32;
        let mut animating = enter_t < 1.0;
        // Exit: accelerate off the edge, or fade when replaced.
        if let Some((at, over)) = self.closing {
            let t = (at.elapsed().as_secs_f32() / motion::tempo(over).as_secs_f32()).min(1.0);
            if over > REPLACE_EXIT {
                offset -= (W + 2.0 * MARGIN + 24.0) * t * t;
                opacity = 1.0 - ((t - 0.66) / 0.34).clamp(0.0, 1.0);
            } else {
                opacity = 1.0 - t;
            }
            if still || t >= 1.0 {
                window.remove_window();
            } else {
                animating = true;
            }
        }
        if animating {
            window.request_animation_frame();
        }
        let card_x = if self.left {
            offset
        } else {
            f32::from(window.viewport_size().width) - offset - W
        };
        self.cutout.set(
            window,
            &[crate::sys::window::Rounded {
                x: card_x,
                y: self.inset,
                w: W,
                h: self.height,
                r: theme::RADIUS_GROUP,
            }],
        );
        let (glyph, tint) = match self.kind {
            Kind::Failure => (Icon::Alert, theme::DANGER),
            Kind::Update => (Icon::Download, theme::ACCENT),
            Kind::Guide => (Icon::Record, theme::DANGER),
            Kind::Saved => (Icon::Check, theme::FG),
            Kind::Countdown(_) => (Icon::Timer, theme::FG),
        };
        let title = match self.kind {
            Kind::Countdown(at) => {
                countdown_title(at.saturating_duration_since(Instant::now())).into()
            }
            _ => self.title.clone(),
        };
        let mut text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .gap(px(TITLE_GAP))
            .child(
                div()
                    .text_size(px(theme::TEXT_HEADLINE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .line_height(px(TITLE_LINE))
                    .text_color(theme::FG)
                    .truncate()
                    .child(title),
            );
        if self.lines > 0 {
            text = text.child(
                div()
                    .text_size(px(theme::TEXT_SMALL))
                    .line_height(px(DETAIL_LINE))
                    .text_color(theme::FG_DIM)
                    .line_clamp(self.lines)
                    .text_ellipsis()
                    .child(self.detail.clone()),
            );
        }
        let reveal = self.file.clone().map(|path| {
            icon_button("notice-reveal", Icon::Folder, false, BUTTON)
                .tooltip(crate::widgets::tip(crate::sys::reveal::LABEL, None))
                .on_click(cx.listener(move |n, _, _, cx| {
                    crate::sys::reveal::reveal(&path);
                    n.leave(EXIT, cx);
                }))
        });
        // The install runs on a thread of its own; a failure arrives
        // through the command pump as a new notice.
        let install = (self.kind == Kind::Update).then(|| {
            icon_button("notice-install", Icon::Download, true, BUTTON)
                .tooltip(crate::widgets::tip("Install", None))
                .on_click(cx.listener(|n, _, _, cx| {
                    if let Err(e) = crate::update::install_offered() {
                        crate::daemon::report_failure("Update failed", e);
                    }
                    n.leave(EXIT, cx);
                }))
        });
        let countdown = matches!(self.kind, Kind::Countdown(_));
        let close = (self.kind != Kind::Guide).then(|| {
            icon_button("notice-close", Icon::Close, false, BUTTON)
                .tooltip(crate::widgets::tip(
                    if countdown { "Cancel" } else { "Close" },
                    None,
                ))
                .on_click(cx.listener(move |n, _, _, cx| {
                    if countdown {
                        COUNTDOWN.fetch_add(1, Ordering::Relaxed);
                    }
                    n.leave(EXIT, cx);
                }))
        });

        let mut card = div()
            .id("notice")
            .absolute()
            .w(px(W))
            .h(px(self.height))
            .p(px(PAD))
            .flex()
            .items_start()
            .gap(px(GAP))
            .rounded(px(theme::RADIUS_GROUP))
            .bg(theme::alpha(theme::BG_ELEV, 0.96))
            .border_1()
            .border_color(theme::HAIRLINE)
            .overflow_hidden()
            .shadow(theme::card_shadow(ease))
            .opacity(opacity)
            .child(
                div()
                    .flex_none()
                    .pt(px((TITLE_LINE - GLYPH) / 2.0))
                    .child(icon(glyph, tint, GLYPH)),
            )
            .child(text)
            .children((buttons(self.kind, self.file.is_some()) > 0).then(|| {
                div()
                    .flex()
                    .flex_none()
                    .gap(px(BUTTON_GAP))
                    .children(reveal)
                    .children(install)
                    .children(close)
            }))
            .on_hover(cx.listener(|n, hovering: &bool, _, cx| {
                n.hovered = *hovering;
                if !*hovering {
                    let hold = n.hold;
                    n.arm(hold, cx);
                }
            }));
        card = if self.left {
            card.left(px(offset))
        } else {
            card.right(px(offset))
        };
        let card = card.top(px(self.inset));
        div().size_full().font_family(theme::FONT).child(card)
    }
}
