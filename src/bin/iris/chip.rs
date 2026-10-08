//! Recording chip: the frosted badge pinned above the recorded area.
//!
//! A small transparent popup with the record dot, a tabular elapsed
//! timer, the pause and stop buttons, and the mic button when the
//! format records audio. It draws a frame when its clock reaches the
//! next second and when the recording's state changes: a recording pays
//! for one chip frame a second, and a paused one for none. On X11 the
//! border thread in sys::record::x11 moves it by XID (x11rb
//! configure_window), so it follows the target window without going
//! through the app event loop. On Windows and macOS it opens above the
//! recorded region and is excluded from screen capture, so it never
//! appears in the recording.

use std::time::{Duration, Instant};

use gpui::*;
use iris_lib::capture::WinRect;

use crate::{icons::Icon, theme};

const CHIP_W: f32 = 208.0;
const CHIP_H: f32 = 36.0;
/// Transparent margin around the pill so its shadow is not clipped
/// by the window bounds.
const CHIP_BLEED: f32 = 16.0;
/// The chip window: the pill plus its bleed on every side.
const WIN_W: f32 = CHIP_W + 2.0 * CHIP_BLEED;
const WIN_H: f32 = CHIP_H + 2.0 * CHIP_BLEED;
/// The pill in the chip window, where a screen that does not blend the
/// window shows it (`sys::window::Cutout`).
const PILL: crate::sys::window::Rounded = crate::sys::window::Rounded {
    x: CHIP_BLEED,
    y: CHIP_BLEED,
    w: CHIP_W,
    h: CHIP_H,
    r: CHIP_H / 2.0,
};

/// The open chip window, for the state setters and `close`.
static CHIP: parking_lot::Mutex<Option<WindowHandle<Chip>>> = parking_lot::Mutex::new(None);

/// Show the recording's pause state on the open chip: its clock stops
/// while paused, since the file has no frames for that span.
pub fn set_paused(cx: &mut App, paused: bool) {
    let now = Instant::now();
    update(cx, move |chip| chip.set_paused(paused, now));
}

/// Show the mic track's state on the open chip.
pub fn set_mic(cx: &mut App, on: bool) {
    update(cx, move |chip| chip.mic = chip.mic.map(|_| on));
}

/// Apply `change` to the open chip and draw it, once the current effect
/// cycle ends. A chip button runs this inside the chip window's event
/// dispatch, where GPUI holds the window and an update of it through
/// its handle fails.
fn update(cx: &mut App, change: impl FnOnce(&mut Chip) + 'static) {
    // Copied out: the update runs with the lock released.
    let Some(handle) = *CHIP.lock() else { return };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |chip, _, cx| {
            change(chip);
            cx.notify();
        });
    });
}

pub struct Chip {
    /// When the elapsed clock read zero, moved later by each pause.
    start: Instant,
    /// When the running pause began; None while recording.
    paused_at: Option<Instant>,
    /// The mic track's state, or None for a format without audio.
    mic: Option<bool>,
    /// The second `text` shows, and its "MM:SS": formatted once a
    /// second, not on every frame.
    shown: u64,
    text: SharedString,
    /// The frame at the clock's next second; None while paused.
    tick: Option<Task<()>>,
    /// The control under the pointer, named in the timer slot.
    hovered: Option<Control>,
    /// The record hotkey, trimmed: empty when none is set.
    record_hotkey: SharedString,
}

impl Chip {
    fn new(mic: Option<bool>, now: Instant) -> Self {
        Self {
            start: now,
            paused_at: None,
            mic,
            shown: 0,
            text: SharedString::new_static("00:00"),
            tick: None,
            hovered: None,
            record_hotkey: SharedString::default(),
        }
    }

    /// Recording time at `now`: wall time since the start, less every
    /// pause.
    fn elapsed(&self, now: Instant) -> Duration {
        self.paused_at
            .unwrap_or(now)
            .saturating_duration_since(self.start)
    }

    fn set_paused(&mut self, paused: bool, now: Instant) {
        match (self.paused_at, paused) {
            (None, true) => {
                self.paused_at = Some(now);
                self.tick = None;
            }
            (Some(at), false) => {
                self.start += now.saturating_duration_since(at);
                self.paused_at = None;
            }
            _ => {}
        }
    }
}

/// Open the chip, replacing one already open. `mic` is the mic track's
/// state, or `None` for a format without audio. `anchor` is the recorded
/// rect in root pixels (`None`: the primary monitor) and `monitors` the
/// root-space monitors, primary first. Returns the chip's X11 window id
/// on X11, 0 elsewhere.
pub fn open(
    cx: &mut App,
    mic: Option<bool>,
    anchor: Option<WinRect>,
    monitors: &[WinRect],
) -> Result<u32, String> {
    close(cx);
    let origin = origin(anchor, monitors, crate::sys::window::root_scale(cx));
    let handle = crate::widgets::open_window(
        cx,
        "Recording - iris",
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(origin.0), px(origin.1)),
                size: size(px(WIN_W), px(WIN_H)),
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
        |window, cx| {
            crate::sys::window::Cutout::default().set(window, &[PILL]);
            let mut chip = Chip::new(mic, Instant::now());
            let keys = iris_lib::config::Config::load().record_hotkey;
            chip.record_hotkey = SharedString::from(keys.trim().to_owned());
            cx.new(|_| chip)
        },
    )
    .map_err(|e| format!("open chip window: {e}"))?;

    let xid = crate::sys::window::prepare_chip(cx, handle.into());
    *CHIP.lock() = Some(handle);
    Ok(xid)
}

/// Logical origin of the chip window for a recording of `rect` (root
/// pixels; `None`: the primary monitor). The pill sits above the rect's
/// top-right corner, below the rect when its monitor has no room above,
/// and inside the rect's top edge when neither side fits: it stays on
/// the rect's monitor and out of the rect whenever the monitor allows.
/// `monitors` are root pixels, primary first; with none, the rect
/// bounds the chip. `scale` is root pixels per logical pixel.
pub(crate) fn origin(rect: Option<WinRect>, monitors: &[WinRect], scale: f32) -> (f32, f32) {
    let logical = |r: &WinRect| {
        let f = |v: f32| v / scale;
        (
            f(r.x as f32),
            f(r.y as f32),
            f(r.width as f32),
            f(r.height as f32),
        )
    };
    let rect = rect
        .or_else(|| monitors.first().copied())
        .unwrap_or(WinRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
    let (x, y, w, h) = logical(&rect);
    let (mx, my, mw, mh) = logical(rect.host(monitors).unwrap_or(&rect));
    // Right-aligned to the rect, never left of it, then kept on the
    // monitor: min before max, so a monitor narrower than the chip
    // pins it to the monitor's left edge.
    let left = (x + w - WIN_W).max(x).min(mx + mw - WIN_W).max(mx);
    let top = if y - WIN_H >= my {
        y - WIN_H
    } else if y + h + WIN_H <= my + mh {
        y + h
    } else {
        y.min(my + mh - WIN_H).max(my)
    };
    (left, top)
}

/// Close the chip window, if open, once the current effect cycle ends:
/// the stop button runs this inside the chip window's event dispatch,
/// as `update` describes.
pub fn close(cx: &mut App) {
    let Some(handle) = CHIP.lock().take() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    });
}

/// The pill's shadow. GPUI paints a shadow over the pill grown by three
/// blur radii and moved by its offset; all of it stays inside
/// CHIP_BLEED, or the window edge cuts it into a visible rectangle over
/// a light background. The bleed cannot grow instead: `origin` puts the
/// window against the recorded rect, so a wider bleed floats the pill
/// further from it.
fn pill_shadow() -> Vec<gpui::BoxShadow> {
    theme::shadow_tight()
}

/// Edge of one chip control: a form control's 24, with a 14px glyph.
const BUTTON: f32 = 24.0;
/// The record dot's diameter.
const DOT: f32 = 9.0;
/// Space between the pill's items.
const GAP: f32 = 6.0;
/// The pill's inner padding: wider before the dot, so the dot sits
/// inside the pill's round end; narrow after the last control, whose
/// hover wash fills the end.
const PAD_LEAD: f32 = 14.0;
const PAD_TRAIL: f32 = 6.0;
/// Space between a control's name and its shortcut in the slot.
const KEYS_GAP: f32 = 6.0;

/// Width of the slot between the dot and the controls, which holds the
/// timer or a hovered control's name. It is fixed for the recording, so
/// a name that replaces the timer never moves a control from under the
/// pointer.
fn slot_w(has_mic: bool) -> f32 {
    let controls = 2 + usize::from(has_mic);
    CHIP_W - PAD_LEAD - PAD_TRAIL - DOT - controls as f32 * BUTTON - (1 + controls) as f32 * GAP
}

/// One chip control: the shared ghost icon button with a hover wash.
fn control(id: &'static str, glyph: Icon) -> Stateful<Div> {
    crate::widgets::icon_button(id, glyph, false, BUTTON)
}

/// A chip control, named in the timer slot while the pointer rests on
/// it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Control {
    Pause,
    Stop,
    Mic,
}

impl Control {
    /// What a click does in the chip's current state.
    fn label(self, paused: bool, mic: Option<bool>) -> &'static str {
        match self {
            Control::Pause if paused => "Resume",
            Control::Pause => "Pause",
            Control::Stop => "Stop",
            Control::Mic if mic == Some(true) => "Mute",
            Control::Mic => "Unmute",
        }
    }
}

/// What the timer slot shows.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Clock,
    /// A control's name, and the record hotkey when that runs it too.
    Control {
        label: &'static str,
        keys: Option<SharedString>,
    },
}

impl Chip {
    /// The clock, or the hovered control's name in the chip's current
    /// state. The record hotkey toggles recording, so while the chip is
    /// open it stops; no other control has a shortcut.
    fn slot(&self) -> Slot {
        let Some(control) = self.hovered else {
            return Slot::Clock;
        };
        let keys = (control == Control::Stop && !self.record_hotkey.is_empty())
            .then(|| self.record_hotkey.clone());
        Slot::Control {
            label: control.label(self.paused_at.is_some(), self.mic),
            keys,
        }
    }

    /// The pointer entered or left `control`. Leaving a control the
    /// pointer already moved off of changes nothing, so a late leave
    /// event does not clear the next control's name.
    fn hover(&mut self, control: Control, hovering: bool) {
        if hovering {
            self.hovered = Some(control);
        } else if self.hovered == Some(control) {
            self.hovered = None;
        }
    }
}

/// Width of `text` in the UI font at `size` and `weight` as GPUI shapes
/// it, logical px.
fn shaped_w(window: &Window, text: &str, size: f32, weight: FontWeight) -> f32 {
    let run = TextRun {
        len: text.len(),
        font: theme::font(weight),
        color: theme::FG.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
        tracking: px(0.),
    };
    window
        .text_system()
        .shape_line(SharedString::from(text.to_owned()), px(size), &[run], None)
        .width
        .into()
}

impl Render for Chip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let elapsed = self.elapsed(Instant::now());
        let secs = elapsed.as_secs();
        if self.shown != secs {
            self.shown = secs;
            self.text = SharedString::from(format!("{:02}:{:02}", secs / 60, secs % 60));
        }
        // The next frame is the clock's next second; a paused clock
        // stands still and draws only when its state changes.
        let paused = self.paused_at.is_some();
        if !paused && self.tick.is_none() {
            let wait = Duration::from_secs(secs + 1).saturating_sub(elapsed);
            self.tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |chip, cx| {
                    chip.tick = None;
                    cx.notify();
                });
            }));
        }
        let mic = self.mic;
        let slot = match self.slot() {
            Slot::Clock => div()
                .font_features(theme::tabular())
                .child(self.text.clone()),
            Slot::Control { label, keys } => {
                // The shortcut shows when it fits beside the name; the
                // slot never grows, and a clipped shortcut reads wrong.
                let keys = keys.filter(|keys| {
                    shaped_w(window, label, theme::TEXT_BODY, FontWeight::MEDIUM)
                        + KEYS_GAP
                        + shaped_w(window, keys, theme::TEXT_SMALL, FontWeight::NORMAL)
                        <= slot_w(mic.is_some())
                });
                div()
                    .flex()
                    .items_center()
                    .gap(px(KEYS_GAP))
                    .child(label)
                    .children(keys.map(|keys| {
                        div()
                            .text_size(px(theme::TEXT_SMALL))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::FG_DIM)
                            .child(keys)
                    }))
            }
        };
        let hover = |control: Control| {
            cx.listener(move |chip: &mut Chip, hovering: &bool, _, cx| {
                chip.hover(control, *hovering);
                cx.notify();
            })
        };

        // The pill inside a window-filling root: taffy places the root
        // element at the window origin whatever its insets, so a bare
        // absolute pill sat in the top-left corner with no bleed on
        // those two sides and its shadow cut off there.
        let pill = div()
            .absolute()
            .left(px(CHIP_BLEED))
            .top(px(CHIP_BLEED))
            .w(px(CHIP_W))
            .h(px(CHIP_H))
            .pl(px(PAD_LEAD))
            .pr(px(PAD_TRAIL))
            .rounded_full()
            .font_family(theme::FONT)
            .bg(theme::alpha(theme::BG_ELEV, 0.92))
            .shadow(pill_shadow())
            .flex()
            .items_center()
            .gap(px(GAP))
            .child(
                div()
                    .flex_none()
                    .size(px(DOT))
                    .rounded_full()
                    .bg(if paused {
                        theme::FG_FAINT
                    } else {
                        theme::DANGER
                    })
                    .opacity(if paused { 0.35 } else { 1.0 }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(theme::TEXT_BODY))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::FG)
                    .child(slot),
            )
            .child(
                control("chip-pause", if paused { Icon::Play } else { Icon::Pause })
                    .on_hover(hover(Control::Pause))
                    .on_click(cx.listener(|_, _, _, cx| {
                        crate::daemon::run(cx, &crate::daemon::Command::RecordPause);
                    })),
            )
            .child(
                control("chip-stop", Icon::Stop)
                    .on_hover(hover(Control::Stop))
                    .on_click(cx.listener(|_, _, _, cx| {
                        crate::daemon::run(cx, &crate::daemon::Command::RecordStop);
                    })),
            )
            .children(mic.map(|on| {
                control("chip-mic", if on { Icon::Mic } else { Icon::MicOff })
                    .on_hover(hover(Control::Mic))
                    .on_click(cx.listener(|_, _, _, cx| {
                        crate::daemon::run(cx, &crate::daemon::Command::RecordMic);
                    }))
            }));
        div().size_full().child(pill)
    }
}

#[cfg(test)]
mod slot_tests;

#[cfg(test)]
mod clock_tests;
#[cfg(test)]
mod tests;
