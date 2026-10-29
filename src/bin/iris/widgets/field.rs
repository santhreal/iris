//! Text fields and the folder field.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::*;

use super::{push_button, ButtonStyle};
use crate::icons::{self, Icon};
use crate::theme;

/// What a text field shows besides its text.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FieldState {
    /// At rest: fill and inner border only.
    Rest,
    /// Being edited: an accent border and ring and a caret at byte
    /// offset `caret` of the text; `invalid` turns the border and ring
    /// destructive red.
    Editing { invalid: bool, caret: usize },
    /// Capturing a key combination: the accent border and ring, no
    /// caret, the text in secondary color (a prompt such as "Type
    /// shortcut").
    Recording,
}

impl FieldState {
    /// The color of the ring around a field in this state, drawn by
    /// [`ringed`], or None at rest.
    pub fn ring(self) -> Option<Rgba> {
        match self {
            FieldState::Rest => None,
            FieldState::Editing { invalid: true, .. } => Some(theme::alpha(theme::DANGER, 0.40)),
            FieldState::Editing { invalid: false, .. } | FieldState::Recording => {
                Some(theme::FOCUS_RING)
            }
        }
    }
}

/// The focus ring's width, logical px.
const RING: f32 = 3.0;

/// `control` inside a focus ring of color `ring`, or none: the field
/// being typed into, or the control Tab moved to, which Space or Return
/// runs. The ring is a 3px border outside the control that takes no
/// layout space and leaves a translucent fill untinted, as a spread
/// shadow under it would not. `radius` is the control's corner radius,
/// None for a pill.
pub fn ringed(control: impl IntoElement, radius: Option<f32>, ring: Option<Rgba>) -> Div {
    let el = div()
        .flex()
        .flex_shrink_0()
        .m(px(-RING))
        .border_3()
        .border_color(ring.map_or(transparent_black(), Hsla::from))
        .child(control);
    match radius {
        Some(r) => el.rounded(px(r + RING)),
        None => el.rounded_full(),
    }
}

/// The index keyboard focus lands on after one Tab or arrow step from
/// `from` among `n` stops, wrapping at both ends. With nothing focused,
/// a forward step lands on the first stop and a backward step on the
/// last. None when there is no stop.
pub fn ring_step(from: Option<usize>, n: usize, forward: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(match (from, forward) {
        (None, true) => 0,
        (None, false) => n - 1,
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
    })
}

/// Where a text field's text starts on screen and the style it is
/// shaped in, recorded each frame the field is drawn, for placing the
/// caret at a click ([`TextOrigin::place`]).
#[derive(Clone, Default)]
pub struct TextOrigin(Rc<RefCell<Option<DrawnText>>>);

/// The left edge of a field's text in the window and its text style.
#[derive(Clone)]
struct DrawnText {
    left: Pixels,
    style: TextStyle,
}

impl TextOrigin {
    /// A zero-width element that records the origin where it is laid
    /// out, ahead of the field's text.
    fn probe(&self) -> impl IntoElement {
        let drawn = self.0.clone();
        canvas(
            move |bounds, window, _| {
                *drawn.borrow_mut() = Some(DrawnText {
                    left: bounds.left(),
                    style: window.text_style(),
                });
            },
            |_, _, _, _| {},
        )
        .w(px(0.))
        .h(px(0.))
        .flex_shrink_0()
    }

    /// Move the caret of `line`, the field's text, to the character
    /// boundary nearest where `click` pressed. A keyboard click, or a
    /// field not yet drawn, leaves the caret.
    pub fn place(&self, line: &mut Line, click: &ClickEvent, window: &Window) {
        let at = match click {
            ClickEvent::Mouse(e) => e.down.position,
            ClickEvent::Touch(e) => e.position,
            ClickEvent::Keyboard(_) => return,
        };
        let Some(DrawnText { left, style }) = self.0.borrow().clone() else {
            return;
        };
        let text = SharedString::from(line.text().to_string());
        let shaped = window.text_system().shape_line(
            text.clone(),
            style.font_size.to_pixels(window.rem_size()),
            &[style.to_run(text.len())],
            None,
        );
        line.place(f32::from(at.x - left), |s| {
            f32::from(shaped.x_for_index(s.len()))
        });
    }
}

/// Single-line text field box, CONTROL_H tall: a dark fill with a 1px
/// inner border, and a caret while editing. The caller draws the ring
/// of `state` ([`FieldState::ring`]) with [`ringed`], has the text
/// buffer, and commits it (Return or blur). With `origin`, the field
/// records where its text starts.
pub fn text_field(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    state: FieldState,
    origin: Option<&TextOrigin>,
) -> Stateful<Div> {
    let border = match state {
        FieldState::Rest => theme::FIELD_BORDER,
        FieldState::Editing { invalid: true, .. } => theme::DANGER,
        FieldState::Editing { invalid: false, .. } | FieldState::Recording => theme::ACCENT,
    };
    let el = div()
        .id(id)
        .h(px(theme::CONTROL_H))
        .px(px(7.))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .rounded(px(theme::RADIUS_CONTROL))
        .bg(theme::FIELD_BG)
        .border_1()
        .border_color(border)
        .text_size(px(theme::TEXT_BODY))
        .text_color(if state == FieldState::Recording {
            theme::FG_DIM
        } else {
            theme::FG
        })
        .cursor_text()
        .children(origin.map(TextOrigin::probe));
    let text = text.into();
    let FieldState::Editing { caret, .. } = state else {
        return el.child(text);
    };
    // The caret sits at the boundary between the text before it and
    // the text after it, in a zero-width slot: moving it never shifts
    // the text.
    let (before, after) = text.split_at(caret.min(text.len()));
    let bar = div().relative().w(px(0.)).h(px(15.)).flex_shrink_0().child(
        div()
            .absolute()
            .top_0()
            .left(px(0.5))
            .w(px(1.5))
            .h(px(15.))
            .bg(theme::ACCENT),
    );
    let mut el = el.child(SharedString::from(before.to_string())).child(bar);
    if !after.is_empty() {
        el = el.child(SharedString::from(after.to_string()));
    }
    el
}

/// A folder row control: a folder glyph and the path, middle-truncated
/// to `width` (the head and the folder name stay readable), then a
/// "Choose…" button that opens the platform's directory picker
/// ([`choose_folder`]), inside a focus ring of color `ring` ([`ringed`]).
pub fn folder_field(
    id: &'static str,
    shown: impl Into<SharedString>,
    width: f32,
    ring: Option<Rgba>,
    on_pick: impl Fn(Result<PathBuf, String>, &mut Window, &mut App) + 'static,
) -> Div {
    let on_pick = Rc::new(on_pick);
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(10.))
        .child(
            div()
                .w(px(width))
                .flex()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .child(icons::icon(Icon::Folder, theme::FG_DIM, theme::ICON_ROW))
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis_middle()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG_DIM)
                        .child(shown.into()),
                ),
        )
        .child(ringed(
            push_button(id, "Choose\u{2026}", ButtonStyle::Bordered).on_click(
                move |_, window, cx| {
                    let on_pick = on_pick.clone();
                    choose_folder(window, cx, move |picked, window, cx| {
                        on_pick(picked, window, cx)
                    });
                },
            ),
            Some(theme::RADIUS_CONTROL),
            ring,
        ))
}

/// Open the platform's directory picker. A chosen directory runs
/// `on_pick(Ok(dir))`; a picker that fails to open runs
/// `on_pick(Err(message))`; a cancelled picker runs nothing.
pub fn choose_folder(
    window: &mut Window,
    cx: &mut App,
    on_pick: impl FnOnce(Result<PathBuf, String>, &mut Window, &mut App) + 'static,
) {
    let rx = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some("Choose".into()),
    });
    window
        .spawn(cx, async move |cx| {
            let picked = match rx.await {
                Ok(Ok(Some(mut dirs))) => dirs.pop().map(Ok),
                Ok(Ok(None)) | Err(_) => None,
                Ok(Err(e)) => Some(Err(format!("folder picker: {e}"))),
            };
            if let Some(picked) = picked {
                cx.update(|window, cx| on_pick(picked, window, cx)).ok();
            }
        })
        .detach();
}

/// The text of a one-line entry and its caret, a byte offset on a
/// character boundary of the text.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Line {
    text: String,
    caret: usize,
}

impl Line {
    /// `text` with the caret after its last character.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Line {
            caret: text.len(),
            text,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret's byte offset in the text.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The text before and after the caret.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.caret)
    }

    /// Move the caret to the character boundary nearest `x`, a distance
    /// from the start of the text, where `advance` gives the width of
    /// each text that starts the line. A tie goes to the earlier
    /// boundary.
    pub fn place(&mut self, x: f32, advance: impl Fn(&str) -> f32) {
        let mut best = (0, x.abs());
        for (i, c) in self.text.char_indices() {
            let end = i + c.len_utf8();
            let at = advance(&self.text[..end]);
            let off = (at - x).abs();
            if off < best.1 {
                best = (end, off);
            } else if at > x {
                break;
            }
        }
        self.caret = best.0;
    }

    /// Move the caret to `to`. Returns whether it moved.
    fn move_to(&mut self, to: usize) -> bool {
        let moved = to != self.caret;
        self.caret = to;
        moved
    }

    /// Remove the text between the caret and `to`, on either side of
    /// it. Returns whether anything was removed.
    fn delete_to(&mut self, to: usize) -> bool {
        let (from, to) = (self.caret.min(to), self.caret.max(to));
        self.text.replace_range(from..to, "");
        self.caret = from;
        from < to
    }

    /// Insert `text` at the caret and move the caret past it. Returns
    /// whether anything was inserted.
    fn insert(&mut self, text: &str) -> bool {
        self.text.insert_str(self.caret, text);
        self.caret += text.len();
        !text.is_empty()
    }

    /// The offset of the character boundary before the caret.
    fn prev_char(&self) -> usize {
        self.text[..self.caret]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    /// The offset of the character boundary after the caret.
    fn next_char(&self) -> usize {
        self.text[self.caret..]
            .chars()
            .next()
            .map_or(self.caret, |c| self.caret + c.len_utf8())
    }

    /// The start of the word before the caret: back over the run of
    /// characters that are not letters or digits, then over the run of
    /// letters and digits before it, as readline's `backward-word`.
    fn word_start(&self) -> usize {
        let before = &self.text[..self.caret];
        let word_end = before.trim_end_matches(|c: char| !c.is_alphanumeric());
        word_end.trim_end_matches(char::is_alphanumeric).len()
    }

    /// The end of the word after the caret: forward over the run of
    /// characters that are not letters or digits, then over the run of
    /// letters and digits after it, as readline's `forward-word`.
    fn word_end(&self) -> usize {
        let after = &self.text[self.caret..];
        let word = after.trim_start_matches(|c: char| !c.is_alphanumeric());
        let rest = word.trim_start_matches(char::is_alphanumeric);
        self.text.len() - rest.len()
    }
}

/// What a key press did to a one-line entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineKey {
    /// The key is not one the entry takes, or took nothing: no text to
    /// remove, no room to move, an empty paste.
    Ignored,
    /// The caret moved; the text is unchanged.
    Moved,
    /// The text changed.
    Edited,
}

/// Apply a key press to a one-line text entry at its caret.
///
/// Left and Right move the caret a character, with Ctrl or Alt a word,
/// and with Cmd to the start or end; Home and End move it to the start
/// or end. Backspace removes the character before the caret, with Ctrl
/// or Alt the word before it, and with Cmd everything before it; Delete
/// removes the character after the caret, with Ctrl or Alt the word
/// after it. Ctrl+V or Cmd+V inserts the text `clipboard` returns as
/// one line, and a key that types a character without Ctrl or Cmd
/// inserts it. Shift moves the caret as the key alone does: the entry
/// has no selection. The caller restarts its caret on a move or edit,
/// and clears a rejection on an edit.
pub fn edit_line(
    line: &mut Line,
    keystroke: &Keystroke,
    clipboard: impl FnOnce() -> Option<String>,
) -> LineKey {
    let mods = &keystroke.modifiers;
    let meta = mods.control || mods.platform;
    let word = mods.control || mods.alt;
    let (moved, edited) = match keystroke.key.as_str() {
        "left" if mods.platform => (line.move_to(0), false),
        "left" if word => (line.move_to(line.word_start()), false),
        "left" => (line.move_to(line.prev_char()), false),
        "right" if mods.platform => (line.move_to(line.text.len()), false),
        "right" if word => (line.move_to(line.word_end()), false),
        "right" => (line.move_to(line.next_char()), false),
        "home" => (line.move_to(0), false),
        "end" => (line.move_to(line.text.len()), false),
        "backspace" if mods.platform => (false, line.delete_to(0)),
        "backspace" if word => (false, line.delete_to(line.word_start())),
        "backspace" => (false, line.delete_to(line.prev_char())),
        "delete" if word => (false, line.delete_to(line.word_end())),
        "delete" => (false, line.delete_to(line.next_char())),
        "v" if meta => (
            false,
            clipboard().is_some_and(|text| line.insert(&one_line(&text))),
        ),
        _ => match (&keystroke.key_char, meta) {
            (Some(ch), false) => (false, line.insert(ch)),
            _ => (false, false),
        },
    };
    match (moved, edited) {
        (_, true) => LineKey::Edited,
        (true, false) => LineKey::Moved,
        (false, false) => LineKey::Ignored,
    }
}

/// Pasted `text` as one line: each line break or tab is a space, a
/// trailing line break is dropped, and other control characters are
/// removed.
fn one_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for (i, part) in text.lines().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        line.extend(part.chars().filter_map(|c| match c {
            '\t' | '\r' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        }));
    }
    line
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};

    use super::{edit_line, ring_step, Line, LineKey};

    // WHY: closes "Tab stalls at an end or skips a stop" for every ring
    // the keyboard moves, Home's tiles and the Settings controls: every
    // stop is reached in both directions, n steps return to the start,
    // and a surface with no stops lands nowhere instead of panicking.
    #[test]
    fn a_ring_step_reaches_every_stop_in_both_directions() {
        assert_eq!(ring_step(None, 0, true), None);
        assert_eq!(ring_step(None, 0, false), None);
        for n in 1..=9 {
            assert_eq!(ring_step(None, n, true), Some(0));
            assert_eq!(ring_step(None, n, false), Some(n - 1));
            for forward in [true, false] {
                let mut seen = vec![false; n];
                let mut at = ring_step(None, n, forward).unwrap();
                for _ in 0..n {
                    assert!(at < n, "n={n}: stop {at} is past the last");
                    seen[at] = true;
                    at = ring_step(Some(at), n, forward).unwrap();
                }
                assert!(seen.iter().all(|&s| s), "n={n} forward={forward}: {seen:?}");
                assert_eq!(Some(at), ring_step(None, n, forward), "n={n}");
            }
        }
    }

    /// A keystroke of `key` typing `key_char`, with each modifier
    /// `mods` names: "ctrl", "alt", "cmd", "shift".
    fn key(key: &str, key_char: Option<&str>, mods: &str) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control: mods.contains("ctrl"),
                alt: mods.contains("alt"),
                platform: mods.contains("cmd"),
                shift: mods.contains("shift"),
                ..Modifiers::default()
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    /// The line `marked` writes, its caret at the `|`.
    fn line(marked: &str) -> Line {
        let caret = marked.find('|').expect("the marked line has a caret");
        let mut line = Line::new(marked.replacen('|', "", 1));
        line.caret = caret;
        line
    }

    /// `line` written with a `|` at its caret.
    fn marked(line: &Line) -> String {
        let (before, after) = line.split();
        format!("{before}|{after}")
    }

    /// The marked line after `start` takes `keystroke` with `clipboard`
    /// on the clipboard, and what the key did.
    fn after(start: &str, keystroke: &Keystroke, clipboard: Option<&str>) -> (String, LineKey) {
        let mut line = line(start);
        let key = edit_line(&mut line, keystroke, || clipboard.map(Into::into));
        (marked(&line), key)
    }

    /// Check each `(start, keystroke, end, outcome)` case.
    fn check(cases: &[(&str, Keystroke, &str, LineKey)]) {
        for (start, keystroke, end, outcome) in cases {
            assert_eq!(
                after(start, keystroke, None),
                (end.to_string(), *outcome),
                "{start:?} + {} {:?}",
                keystroke.key,
                keystroke.modifiers
            );
        }
    }

    /// A paste reaches a one-line entry as one line at the caret, from
    /// Ctrl+V and Cmd+V alike, whatever line breaks, tabs, or control
    /// characters the clipboard holds.
    #[test]
    fn a_paste_inserts_the_clipboard_as_one_line_at_the_caret() {
        for (pasted, line) in [
            ("Launch day", "Launch day"),
            ("Launch day\n", "Launch day"),
            ("Launch day\r\n", "Launch day"),
            ("first\nsecond", "first second"),
            ("first\r\nsecond\r\n", "first second"),
            ("a\tb", "a b"),
            ("bell\u{7}less", "bellless"),
            ("lone\rreturn", "lone return"),
            ("café ✓", "café ✓"),
        ] {
            for mods in ["ctrl", "cmd"] {
                let paste = key("v", Some("v"), mods);
                assert_eq!(
                    after("x |", &paste, Some(pasted)),
                    (format!("x {line}|"), LineKey::Edited),
                    "{pasted:?} with {mods}"
                );
                assert_eq!(
                    after("x |.png", &paste, Some(pasted)),
                    (format!("x {line}|.png"), LineKey::Edited),
                    "{pasted:?} with {mods} before more text"
                );
            }
        }
    }

    /// Keys other than a paste type at the caret, delete, or do nothing;
    /// a paste of nothing edits nothing.
    #[test]
    fn typing_and_backspace_edit_at_the_caret_and_chords_do_not() {
        let cases = [
            (
                "ab|",
                key("c", Some("c"), ""),
                None,
                "abc|",
                LineKey::Edited,
            ),
            (
                "a|c",
                key("b", Some("b"), ""),
                None,
                "ab|c",
                LineKey::Edited,
            ),
            (
                "|b",
                key("a", Some("A"), "shift"),
                None,
                "A|b",
                LineKey::Edited,
            ),
            (
                "ab|",
                key("v", Some("v"), ""),
                Some("zz"),
                "abv|",
                LineKey::Edited,
            ),
            (
                "ab|",
                key("backspace", None, ""),
                None,
                "a|",
                LineKey::Edited,
            ),
            (
                "a|b",
                key("backspace", None, ""),
                None,
                "|b",
                LineKey::Edited,
            ),
            (
                "|ab",
                key("backspace", None, ""),
                None,
                "|ab",
                LineKey::Ignored,
            ),
            ("|", key("backspace", None, ""), None, "|", LineKey::Ignored),
            (
                "ab|",
                key("c", Some("c"), "ctrl"),
                None,
                "ab|",
                LineKey::Ignored,
            ),
            (
                "ab|",
                key("a", Some("a"), "cmd"),
                None,
                "ab|",
                LineKey::Ignored,
            ),
            (
                "ab|",
                key("v", Some("v"), "ctrl"),
                None,
                "ab|",
                LineKey::Ignored,
            ),
            (
                "ab|",
                key("v", Some("v"), "ctrl"),
                Some(""),
                "ab|",
                LineKey::Ignored,
            ),
            (
                "ab|",
                key("v", Some("v"), "ctrl"),
                Some("\n"),
                "ab|",
                LineKey::Ignored,
            ),
            ("a|b", key("up", None, ""), None, "a|b", LineKey::Ignored),
            ("a|b", key("down", None, ""), None, "a|b", LineKey::Ignored),
        ];
        for (start, keystroke, clipboard, end, outcome) in cases {
            assert_eq!(
                after(start, &keystroke, clipboard),
                (end.to_string(), outcome),
                "{start:?} + {keystroke:?} with {clipboard:?}"
            );
        }
    }

    /// Backspace with Ctrl or Alt removes the word before the caret and
    /// what follows it up to the caret, a word being letters and digits;
    /// with Cmd it removes everything before the caret. A plain Backspace
    /// removes one character, whatever it is. Text after the caret stays.
    #[test]
    fn a_modified_backspace_removes_the_word_or_the_text_before_the_caret() {
        let backspace = |mods| key("backspace", None, mods);
        check(&[
            (
                "hello world|",
                backspace("ctrl"),
                "hello |",
                LineKey::Edited,
            ),
            ("hello world|", backspace("alt"), "hello |", LineKey::Edited),
            (
                "hello world  |",
                backspace("ctrl"),
                "hello |",
                LineKey::Edited,
            ),
            (
                "hello wor|ld",
                backspace("ctrl"),
                "hello |ld",
                LineKey::Edited,
            ),
            ("hello |world", backspace("alt"), "|world", LineKey::Edited),
            (
                "~/Pictures/Screenshots/|",
                backspace("ctrl"),
                "~/Pictures/|",
                LineKey::Edited,
            ),
            (
                "{date}_{time}|",
                backspace("alt"),
                "{date}_{|",
                LineKey::Edited,
            ),
            ("naïve café|", backspace("ctrl"), "naïve |", LineKey::Edited),
            ("shot2024|", backspace("ctrl"), "|", LineKey::Edited),
            ("--- |", backspace("ctrl"), "|", LineKey::Edited),
            ("|", backspace("ctrl"), "|", LineKey::Ignored),
            ("|ab", backspace("alt"), "|ab", LineKey::Ignored),
            ("ab cd|", backspace("cmd"), "|", LineKey::Edited),
            ("ab |cd", backspace("cmd"), "|cd", LineKey::Edited),
            ("|cd", backspace("cmd"), "|cd", LineKey::Ignored),
            ("café|", backspace(""), "caf|", LineKey::Edited),
            ("caf|é", backspace(""), "ca|é", LineKey::Edited),
        ]);
    }

    /// Delete removes the character after the caret, and with Ctrl or
    /// Alt the word after it and what precedes that word.
    #[test]
    fn delete_removes_the_character_or_the_word_after_the_caret() {
        let delete = |mods| key("delete", None, mods);
        check(&[
            ("a|bc", delete(""), "a|c", LineKey::Edited),
            ("caf|é!", delete(""), "caf|!", LineKey::Edited),
            ("abc|", delete(""), "abc|", LineKey::Ignored),
            ("|hello world", delete("ctrl"), "| world", LineKey::Edited),
            ("hello| world", delete("alt"), "hello|", LineKey::Edited),
            (
                "{date}|_{time}.png",
                delete("ctrl"),
                "{date}|}.png",
                LineKey::Edited,
            ),
            ("he|llo world", delete("ctrl"), "he| world", LineKey::Edited),
            ("ab|--", delete("alt"), "ab|", LineKey::Edited),
            ("ab|", delete("ctrl"), "ab|", LineKey::Ignored),
        ]);
    }

    /// Left and Right move the caret a character, with Ctrl or Alt a
    /// word, and with Cmd to an end; Home and End move it to an end.
    /// Shift moves it as the key alone does. None of them changes the
    /// text, and a move with no room is ignored.
    #[test]
    fn arrows_home_and_end_move_the_caret_without_editing() {
        let (left, right) = (
            |mods| key("left", None, mods),
            |mods| key("right", None, mods),
        );
        check(&[
            ("ab|c", left(""), "a|bc", LineKey::Moved),
            ("ab|c", left("shift"), "a|bc", LineKey::Moved),
            ("café|", left(""), "caf|é", LineKey::Moved),
            ("|abc", left(""), "|abc", LineKey::Ignored),
            ("a|bc", right(""), "ab|c", LineKey::Moved),
            ("caf|é", right(""), "café|", LineKey::Moved),
            ("abc|", right("shift"), "abc|", LineKey::Ignored),
            ("hello world|", left("ctrl"), "hello |world", LineKey::Moved),
            ("hello |world", left("alt"), "|hello world", LineKey::Moved),
            ("hello wo|rld", left("ctrl"), "hello |world", LineKey::Moved),
            ("~/Pictures/|", left("ctrl"), "~/|Pictures/", LineKey::Moved),
            ("--|", left("ctrl"), "|--", LineKey::Moved),
            (
                "|hello world",
                right("ctrl"),
                "hello| world",
                LineKey::Moved,
            ),
            ("hello| world", right("alt"), "hello world|", LineKey::Moved),
            (
                "{date}|_{time}",
                right("ctrl"),
                "{date}_{time|}",
                LineKey::Moved,
            ),
            ("naï|ve café", right("ctrl"), "naïve| café", LineKey::Moved),
            ("ab|--", right("alt"), "ab--|", LineKey::Moved),
            ("ab cd|", left("cmd"), "|ab cd", LineKey::Moved),
            ("|ab cd", right("cmd"), "ab cd|", LineKey::Moved),
            ("|abc", left("cmd"), "|abc", LineKey::Ignored),
            ("ab|c", key("home", None, ""), "|abc", LineKey::Moved),
            ("ab|c", key("end", None, "shift"), "abc|", LineKey::Moved),
            ("|abc", key("home", None, ""), "|abc", LineKey::Ignored),
            ("abc|", key("end", None, ""), "abc|", LineKey::Ignored),
        ]);
    }

    // WHY: closes "a caret key lands between the bytes of a character
    // or reports the wrong outcome" for every key the entry takes, with
    // every modifier, from every caret position of text with one-, two-,
    // three-, and four-byte characters. The caret stays on a character
    // boundary of the text, a move leaves the text, an edit changes it,
    // and an ignored key changes nothing, so the caller's caret restart
    // and rejection reset follow what happened.
    #[test]
    fn every_key_leaves_the_caret_on_a_character_boundary() {
        let keys = [
            "left",
            "right",
            "home",
            "end",
            "backspace",
            "delete",
            "v",
            "x",
        ];
        let mods = ["", "shift", "ctrl", "alt", "cmd", "ctrl shift", "alt shift"];
        for text in ["", "a", "ab cd", "naïve—café ✓ 𝄞x", "--_{date}"] {
            let carets = text.char_indices().map(|(i, _)| i).chain([text.len()]);
            for caret in carets {
                for name in keys {
                    for mods in mods {
                        let mut line = Line::new(text);
                        line.caret = caret;
                        let typed = (name == "x").then_some("é");
                        let keystroke = key(name, typed, mods);
                        let outcome = edit_line(&mut line, &keystroke, || Some("ü\n".into()));
                        let at = format!("{text:?} at {caret} + {mods} {name}");
                        assert!(
                            line.text().is_char_boundary(line.caret()),
                            "{at}: caret {} is inside a character",
                            line.caret()
                        );
                        match outcome {
                            LineKey::Ignored => {
                                assert_eq!((line.text(), line.caret()), (text, caret), "{at}")
                            }
                            LineKey::Moved => {
                                assert_eq!(line.text(), text, "{at}");
                                assert_ne!(line.caret(), caret, "{at}");
                            }
                            LineKey::Edited => assert_ne!(line.text(), text, "{at}"),
                        }
                    }
                }
            }
        }
    }

    // WHY: closes "a click puts the caret on the wrong side of the
    // character it lands on" for both entries that place the caret at a
    // click. The caret goes to the nearest boundary measured by the
    // text's own widths, a tie goes left, a click before or past the
    // text lands at its start or end, and a multibyte character is never
    // split. It does not cover where a click lands on screen; the X11
    // tests in history.rs and settings_paste.rs do.
    #[test]
    fn a_click_places_the_caret_at_the_nearest_boundary() {
        // Narrow i, wide m, and ten pixels for every other character.
        let advance = |s: &str| {
            s.chars()
                .map(|c| match c {
                    'i' => 4.0,
                    'm' => 16.0,
                    _ => 10.0,
                })
                .sum::<f32>()
        };
        for (start, x, end) in [
            ("|", 30.0, "|"),
            ("ab cd|", -5.0, "|ab cd"),
            ("ab cd|", 4.0, "|ab cd"),
            ("ab cd|", 5.0, "|ab cd"),
            ("ab cd|", 6.0, "a|b cd"),
            ("|ab cd", 26.0, "ab |cd"),
            ("|ab cd", 46.0, "ab cd|"),
            ("|ab cd", 400.0, "ab cd|"),
            ("mim|", 17.0, "m|im"),
            ("mim|", 19.0, "mi|m"),
            ("mim|", 28.0, "mi|m"),
            ("mim|", 29.0, "mim|"),
            ("|mm", 20.0, "m|m"),
            ("ii|", 5.0, "i|i"),
            ("é✓𝄞|", 12.0, "é|✓𝄞"),
            ("|é✓𝄞", 25.0, "é✓|𝄞"),
            ("|é✓𝄞", 26.0, "é✓𝄞|"),
        ] {
            let mut placed = line(start);
            placed.place(x, advance);
            assert_eq!(marked(&placed), end, "{start:?} at {x}");
        }
    }
}
