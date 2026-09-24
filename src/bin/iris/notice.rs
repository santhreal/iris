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

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::*;

use crate::icons::{icon, Icon};
use crate::widgets::icon_button;
use crate::{motion, theme};

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
const BUTTON: f32 = 26.0;
const BUTTON_GAP: f32 = 4.0;
const GAP: f32 = 10.0;
const TITLE_LINE: f32 = 18.0;
const TITLE_GAP: f32 = 2.0;
const DETAIL_LINE: f32 = 16.0;
/// A longer detail ends in an ellipsis on its last line.
const MAX_LINES: usize = 3;
/// A failure stays at least this long: it is read, not glanced at.
const FAILURE_HOLD: Duration = Duration::from_secs(8);
const EXIT: Duration = Duration::from_millis(300);
/// Replaced by a newer notice or a toast: a fast fade, not the slide.
const REPLACE_EXIT: Duration = Duration::from_millis(180);

/// The live notice, so a newer one replaces it.
static NOTICE: parking_lot::Mutex<Option<WindowHandle<Notice>>> = parking_lot::Mutex::new(None);

/// A file was written: its name, and Show in Folder.
pub fn saved(cx: &mut App, title: &str, path: &Path) {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    show(cx, title, name, Some(path.to_path_buf()), false);
}

/// `what` failed with `err`.
pub fn failed(cx: &mut App, what: &str, err: &str) {
    show(cx, what, one_paragraph(err), None, true);
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

fn show(cx: &mut App, title: &str, detail: String, file: Option<PathBuf>, failure: bool) {
    if let Err(e) = open(cx, title, detail, file, failure) {
        iris_lib::ilog!("iris: notice: {e}");
    }
}

pub struct Notice {
    title: SharedString,
    detail: SharedString,
    /// Detail lines the card is sized for; 0 without a detail.
    lines: usize,
    file: Option<PathBuf>,
    failure: bool,
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
}

/// Width of the detail text beside the glyph and `buttons` buttons.
fn text_width(buttons: usize) -> f32 {
    let row = buttons as f32 * BUTTON + buttons.saturating_sub(1) as f32 * BUTTON_GAP;
    W - 2.0 * (BORDER + PAD) - GLYPH - GAP - row - GAP
}

/// Lines `text` wraps to at `cols` characters a line: greedy, at
/// whitespace, with a word wider than a line broken across lines.
fn wrapped_lines(text: &str, cols: usize) -> usize {
    let cols = cols.max(1);
    let mut lines = 1;
    let mut col = 0;
    for word in text.split_whitespace() {
        let mut n = word.chars().count();
        if col > 0 && col + 1 + n <= cols {
            col += 1 + n;
            continue;
        }
        if col > 0 {
            lines += 1;
        }
        while n > cols {
            lines += 1;
            n -= cols;
        }
        col = n;
    }
    lines
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
    failure: bool,
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
    let hold = if failure {
        hold.max(FAILURE_HOLD)
    } else {
        hold
    };
    let buttons = 1 + usize::from(file.is_some());
    // Monospace: the detail wraps at a known column, so the card height
    // is known before layout.
    let cols = (text_width(buttons) / theme::SMALL_ADVANCE).floor() as usize;
    let lines = if detail.trim().is_empty() {
        0
    } else {
        wrapped_lines(&detail, cols).min(MAX_LINES)
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
        failure,
        height,
        card,
        inset: card.1 - win.1,
        left,
        hold,
        opened: None,
        closing: None,
        hovered: false,
        armed: 0,
    };
    let handle = cx
        .open_window(
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
                app_id: Some("dev.iris.notice".to_string()),
                window_min_size: None,
                window_decorations: Some(WindowDecorations::Client),
                tabbing_identifier: None,
            },
            |_, cx| cx.new(|_| notice),
        )
        .map_err(|e| format!("open notice window: {e}"))?;
    let previous = NOTICE.lock().replace(handle);
    if let Some(previous) = previous {
        let _ = previous.update(cx, |n, _, cx| n.leave(REPLACE_EXIT, cx));
    }
    handle
        .update(cx, |n, _, cx| {
            let wait = motion::tempo(motion::ENTER) + n.hold;
            n.arm(wait, cx);
        })
        .map_err(|e| format!("arm notice dismiss: {e}"))?;
    Ok(())
}

impl Notice {
    /// Leave after `wait`, or later while the pointer rests on the card.
    fn arm(&mut self, wait: Duration, cx: &mut Context<Self>) {
        if self.closing.is_some() {
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
        // Entrance: decelerating hard into the corner, as the toast.
        let opened = *self.opened.get_or_insert_with(Instant::now);
        let enter_t =
            (opened.elapsed().as_secs_f32() / motion::tempo(motion::ENTER).as_secs_f32()).min(1.0);
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
            if t >= 1.0 {
                window.remove_window();
            } else {
                animating = true;
            }
        }
        if animating {
            window.request_animation_frame();
        }

        let (glyph, tint) = if self.failure {
            (Icon::Alert, theme::DANGER)
        } else {
            (Icon::Check, theme::FG)
        };
        let mut text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .gap(px(TITLE_GAP))
            .child(
                div()
                    .text_size(px(theme::TEXT_TITLE))
                    .line_height(px(TITLE_LINE))
                    .text_color(theme::FG)
                    .truncate()
                    .child(self.title.clone()),
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
            icon_button("notice-reveal", Icon::Folder, false, BUTTON).on_click(cx.listener(
                move |n, _, _, cx| {
                    crate::sys::reveal::reveal(&path);
                    n.leave(EXIT, cx);
                },
            ))
        });
        let close = icon_button("notice-close", Icon::Close, false, BUTTON)
            .on_click(cx.listener(|n, _, _, cx| n.leave(EXIT, cx)));

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
            .child(div().flex_none().pt(px(1.)).child(icon(glyph, tint, GLYPH)))
            .child(text)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(BUTTON_GAP))
                    .children(reveal)
                    .child(close),
            )
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
