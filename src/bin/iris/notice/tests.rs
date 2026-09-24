// WHY: three classes are closed here. "The card is sized for text it
// does not hold": the height is fixed before layout from the wrapped
// line count, so a count below the real one clips the detail and a
// count above it leaves a hole. "The card covers the toast": a notice
// that lands on a live toast hides the capture the toast holds, and a
// notice window that reaches into the toast card, or a toast window
// that reaches into the notice card, takes the clicks meant for the
// card under it. "A card window cuts its own shadow": a window edge
// inside the card shadow's reach, off the screen edge, ends the blur in
// a hard line; a card stacked beyond a toast once cut its shadow 12px
// below itself. The placement cases run every corner, every card
// height, and toast sizes from a sliver to the largest card. Not
// covered: GPUI's own wrap of a line the estimate matches, which the
// monospace advance makes column-exact, and the window manager
// honoring the requested origin.

use std::prelude::v1::test;

use super::*;

const SCREENS: [Rect; 2] = [
    (0.0, 0.0, 1920.0, 1080.0),
    (-2560.0, -300.0, 2560.0, 1440.0),
];
const CORNERS: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

/// The card's rect in the bare corner of `screen`.
fn corner(screen: Rect, left: bool, top: bool, height: f32) -> Rect {
    let (sx, sy, sw, sh) = screen;
    let x = if left {
        sx + MARGIN
    } else {
        sx + sw - MARGIN - W
    };
    let y = if top {
        sy + MARGIN
    } else {
        sy + sh - MARGIN - height
    };
    (x, y, W, height)
}

/// A `w`x`h` toast card at rest in the corner, as the toast places it.
fn toast_card(screen: Rect, left: bool, top: bool, w: f32, h: f32) -> Rect {
    let (sx, sy, sw, sh) = screen;
    let m = crate::stage::MARGIN;
    let x = if left { sx + m } else { sx + sw - m - w };
    let y = if top { sy + m } else { sy + sh - m - h };
    (x, y, w, h)
}

/// The toast's window around its card `t`, as the toast opens it:
/// shadow room on the inner sides, the screen margin on the outer ones.
fn toast_window(t: Rect, left: bool, top: bool) -> Rect {
    let (b, m) = (crate::stage::BLEED, crate::stage::MARGIN);
    let x = if left { t.0 - m } else { t.0 - b };
    let y = if top { t.1 - m } else { t.1 - b };
    (x, y, t.2 + b + m, t.3 + b + m)
}

/// Room between `card` and each edge of `window`: left, top, right,
/// bottom.
fn room(window: Rect, card: Rect) -> [f32; 4] {
    [
        card.0 - window.0,
        card.1 - window.1,
        window.0 + window.2 - (card.0 + card.2),
        window.1 + window.3 - (card.1 + card.3),
    ]
}

/// Which edges of `window` lie on the edge of `screen`: left, top,
/// right, bottom.
fn on_screen_edge(window: Rect, screen: Rect) -> [bool; 4] {
    [
        window.0 == screen.0,
        window.1 == screen.1,
        window.0 + window.2 == screen.0 + screen.2,
        window.1 + window.3 == screen.1 + screen.3,
    ]
}

#[test]
fn wraps_greedily_at_whitespace() {
    let cases: [(&str, usize, usize); 9] = [
        ("", 10, 1),
        ("short", 10, 1),
        ("exactly10c", 10, 1),
        ("two words", 9, 1),
        ("two words", 8, 2),
        ("aaaa bbbb cccc", 9, 2),
        ("aaaa  bbbb\ncccc", 9, 2),
        ("abcdefghijklmnopqrstuvwxy", 10, 3),
        ("ab abcdefghijklmnopqrst cd", 10, 4),
    ];
    for (text, cols, want) in cases {
        assert_eq!(
            wrapped_lines(text, cols),
            want,
            "{text:?} at {cols} columns"
        );
    }
}

#[test]
fn zero_columns_count_one_character_a_line() {
    assert_eq!(wrapped_lines("abc", 0), 3);
}

#[test]
fn an_error_becomes_one_paragraph() {
    // A hard break the estimate cannot see would add a line GPUI
    // draws and the card has no room for.
    let err = "ffmpeg exited 1:\n  [in#0] Error opening input\r\n\tfile: x.mkv  ";
    let p = one_paragraph(err);
    assert_eq!(p, "ffmpeg exited 1: [in#0] Error opening input file: x.mkv");
    assert!(!p.contains(['\n', '\r', '\t']));
}

#[test]
fn card_holds_its_text_and_buttons_with_no_slack() {
    let mut last = 0.0;
    for lines in 0..=MAX_LINES {
        let inner = card_height(lines) - 2.0 * (BORDER + PAD);
        let text = match lines {
            0 => TITLE_LINE,
            n => TITLE_LINE + TITLE_GAP + n as f32 * DETAIL_LINE,
        };
        assert_eq!(inner, text.max(BUTTON), "{lines} lines");
        assert!(card_height(lines) > last, "height grows with {lines} lines");
        last = card_height(lines);
    }
}

#[test]
fn detail_column_fits_a_recording_name_beside_both_buttons() {
    let cols = |buttons| (text_width(buttons) / theme::SMALL_ADVANCE).floor() as usize;
    // The longest name unique_recording_path makes, but the millis
    // fallback: a date, a time, a 3-digit suffix, and the extension.
    assert_eq!(wrapped_lines("2026-08-09_14-02-11_999.webm", cols(2)), 1);
    assert!(cols(2) < cols(1), "the reveal button narrows the column");
    assert!(cols(2) >= 36, "{} columns", cols(2));
}

#[test]
fn overlap_needs_shared_area() {
    let a = (0.0, 0.0, 10.0, 10.0);
    assert!(overlaps(a, a));
    assert!(overlaps(a, (9.0, 9.0, 5.0, 5.0)));
    assert!(overlaps(a, (-5.0, 2.0, 30.0, 1.0)), "a strip across it");
    for touching in [
        (10.0, 0.0, 5.0, 10.0),
        (0.0, 10.0, 10.0, 5.0),
        (-5.0, -5.0, 5.0, 5.0),
    ] {
        assert!(!overlaps(a, touching), "{touching:?}");
        assert!(!overlaps(touching, a), "{touching:?}");
    }
}

#[test]
fn lands_in_its_corner_inside_the_screen() {
    for screen in SCREENS {
        for (left, top) in CORNERS {
            for lines in 0..=MAX_LINES {
                let h = card_height(lines);
                let (window, card) = place(screen, left, top, h, None);
                assert_eq!(
                    card,
                    corner(screen, left, top, h),
                    "{screen:?} {left} {top}"
                );
                let (sx, sy, sw, sh) = screen;
                let (ox, oy, ww, wh) = window;
                assert_eq!((ww, wh), (W + BLEED + MARGIN, h + BLEED + MARGIN));
                assert!(ox >= sx && ox + ww <= sx + sw, "x {ox} on {screen:?}");
                assert!(oy >= sy && oy + wh <= sy + sh, "y {oy} on {screen:?}");
            }
        }
    }
}

#[test]
fn never_covers_a_live_toast() {
    for screen in SCREENS {
        for (left, top) in CORNERS {
            for (tw, th) in [(200.0, 140.0), (40.0, 140.0), (200.0, 12.0), (1.0, 1.0)] {
                let toast = toast_card(screen, left, top, tw, th);
                for lines in 0..=MAX_LINES {
                    let h = card_height(lines);
                    let (window, card) = place(screen, left, top, h, Some(toast));
                    let at = format!("{screen:?} left={left} top={top} toast {tw}x{th}");
                    assert!(!overlaps(card, toast), "{at}: {card:?} covers {toast:?}");
                    assert!(
                        !overlaps(window, toast),
                        "{at}: window {window:?} reaches into {toast:?}"
                    );
                    let beside = toast_window(toast, left, top);
                    assert!(
                        !overlaps(beside, card),
                        "{at}: toast window {beside:?} reaches into {card:?}"
                    );
                    assert_eq!(card.0, corner(screen, left, top, h).0, "{at}: same column");
                    let gap = if top {
                        card.1 - (toast.1 + toast.3)
                    } else {
                        toast.1 - (card.1 + card.3)
                    };
                    assert_eq!(gap, STACK_GAP, "{at}: stacked beyond the toast");
                }
            }
        }
    }
}

#[test]
fn every_window_holds_its_card_shadow() {
    let reach = theme::shadow_reach(&theme::card_shadow(1.0));
    let toasts = [
        None,
        Some((200.0, 140.0)),
        Some((40.0, 140.0)),
        Some((200.0, 12.0)),
        Some((1.0, 1.0)),
    ];
    for screen in SCREENS {
        for (left, top) in CORNERS {
            for size in toasts {
                let toast = size.map(|(w, h)| toast_card(screen, left, top, w, h));
                for lines in 0..=MAX_LINES {
                    let (window, card) = place(screen, left, top, card_height(lines), toast);
                    let at = format!("{screen:?} left={left} top={top} toast {size:?}");
                    let sides = ["left", "top", "right", "bottom"];
                    let edges = on_screen_edge(window, screen);
                    for ((side, space), edge) in sides.iter().zip(room(window, card)).zip(edges) {
                        if edge {
                            assert_eq!(space, MARGIN, "{at}: {side} at the screen edge");
                        } else {
                            assert!(
                                space >= reach,
                                "{at}: {side} holds {space}px of a {reach}px shadow"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_toast_elsewhere_leaves_the_corner_alone() {
    let (primary, other) = (SCREENS[0], (1920.0, 0.0, 2560.0, 1440.0));
    for (left, top) in CORNERS {
        let h = card_height(2);
        let toast = toast_card(other, left, top, 200.0, 140.0);
        let (_, card) = place(primary, left, top, h, Some(toast));
        assert_eq!(card, corner(primary, left, top, h), "left={left} top={top}");
        // The opposite corner of the same display is elsewhere too.
        let across = toast_card(primary, !left, !top, 200.0, 140.0);
        let (_, card) = place(primary, left, top, h, Some(across));
        assert_eq!(card, corner(primary, left, top, h), "left={left} top={top}");
    }
}
