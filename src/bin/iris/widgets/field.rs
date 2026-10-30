//! Text fields and the folder field.

use std::cell::Cell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::*;

use super::{push_button, ButtonStyle};
use crate::icons::{self, Icon};
use crate::theme;

/// What a text field shows besides its text.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FieldState {
    /// At rest: fill and inner border only.
    Rest,
    /// Being edited: an accent border and ring, a caret at byte offset
    /// `caret` of the text, and the text between `caret` and `anchor`
    /// on the selection fill, with the text moved left by `scroll` px
    /// ([`field_scroll`]); `invalid` turns the border and ring
    /// destructive red.
    Editing {
        invalid: bool,
        caret: usize,
        anchor: usize,
        scroll: f32,
    },
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

/// A text field's padding inside its 1 px border, and the caret bar's
/// width, logical px.
const FIELD_PAD: f32 = 7.0;
const CARET_W: f32 = 2.0;

/// `text` shaped as a text field draws it.
fn shape_field_text(text: &str, window: &Window) -> ShapedLine {
    let mut style = window.text_style();
    style.font_family = theme::FONT.into();
    let text = SharedString::from(text.to_string());
    let run = style.to_run(text.len());
    window
        .text_system()
        .shape_line(text, px(theme::TEXT_BODY), &[run], None)
}

/// How far left an open text field `width` px wide moves the text of
/// `line` to keep its caret in view, from `was`, the distance of the
/// frame before.
pub fn field_scroll(line: &Line, width: f32, was: f32, window: &Window) -> f32 {
    let shaped = shape_field_text(line.text(), window);
    keep_in_view(
        was,
        f32::from(shaped.x_for_index(line.caret())),
        f32::from(shaped.width()),
        width - 2.0 * (1.0 + FIELD_PAD) - CARET_W,
    )
}

/// The scroll, from `was`, that shows a caret `caret_x` px into a text
/// `width` px wide in a view `view` px wide: it moves only as far as
/// the caret is out of view, and never past the start of the text or
/// so far that the view ends past the text's end.
fn keep_in_view(was: f32, caret_x: f32, width: f32, view: f32) -> f32 {
    was.min(caret_x)
        .max(caret_x - view)
        .min(width - view)
        .max(0.0)
}

/// Where a text field's text starts in the window, recorded each frame
/// the field is drawn, for placing the caret at a click
/// ([`TextOrigin::place`]).
#[derive(Clone, Default)]
pub struct TextOrigin(Rc<Cell<Option<Pixels>>>);

impl TextOrigin {
    /// A zero-width element that records the left edge where it is laid
    /// out, ahead of the field's text.
    fn probe(&self) -> impl IntoElement {
        let left = self.0.clone();
        canvas(
            move |bounds, _, _| left.set(Some(bounds.left())),
            |_, _, _, _| {},
        )
        .w(px(0.))
        .h(px(0.))
        .flex_shrink_0()
    }

    /// Move the caret of `line`, the field's text moved left by
    /// `scroll` px, to the character boundary nearest where `click`
    /// pressed, selecting from the anchor with `extend`. A keyboard
    /// click, or a field not yet drawn, leaves the caret.
    pub fn place(
        &self,
        line: &mut Line,
        click: &ClickEvent,
        scroll: f32,
        extend: bool,
        window: &Window,
    ) {
        let at = match click {
            ClickEvent::Mouse(e) => e.down.position,
            ClickEvent::Touch(e) => e.position,
            ClickEvent::Keyboard(_) => return,
        };
        let Some(left) = self.0.get() else {
            return;
        };
        let shaped = shape_field_text(line.text(), window);
        line.place(f32::from(at.x - left) + scroll, extend, |s| {
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
        .px(px(FIELD_PAD))
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
    let FieldState::Editing {
        caret,
        anchor,
        scroll,
        ..
    } = state
    else {
        return el.child(text);
    };
    // The caret sits at the boundary between the text before it and
    // the text after it, in a zero-width slot: moving it never shifts
    // the text unless the caret leaves the view. Selected text sits on
    // the selection fill, the caret at its start or end.
    let (caret, anchor) = (caret.min(text.len()), anchor.min(text.len()));
    let (start, end) = (caret.min(anchor), caret.max(anchor));
    let piece = |range: Range<usize>| SharedString::from(text[range].to_string());
    let bar = div().relative().w(px(0.)).h(px(15.)).flex_shrink_0().child(
        div()
            .absolute()
            .top_0()
            .left(px(0.5))
            .w(px(CARET_W - 0.5))
            .h(px(15.))
            .bg(theme::ACCENT),
    );
    let (lead, trail) = if caret == start {
        (Some(bar), None)
    } else {
        (None, Some(bar))
    };
    let line = div()
        .flex()
        .items_center()
        .flex_shrink_0()
        .ml(px(-scroll))
        .child(piece(0..start))
        .children(lead)
        .children((start < end).then(|| div().bg(theme::TEXT_SELECTION).child(piece(start..end))))
        .children(trail)
        .children((end < text.len()).then(|| piece(end..text.len())));
    // The text clips at the padding, not under the border.
    el.child(
        div()
            .flex()
            .items_center()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .child(line),
    )
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

/// The text of a one-line entry, its caret, and its selection anchor,
/// byte offsets on character boundaries of the text. The selection
/// runs between the anchor and the caret; an anchor at the caret
/// selects nothing.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Line {
    text: String,
    caret: usize,
    anchor: usize,
}

impl Line {
    /// `text` with the caret after its last character and nothing
    /// selected.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        Line {
            caret: text.len(),
            anchor: text.len(),
            text,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The selection anchor's byte offset in the text, at the caret
    /// when nothing is selected.
    pub fn anchor(&self) -> usize {
        self.anchor
    }

    /// The caret's byte offset in the text.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range of the text, empty when nothing is
    /// selected.
    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// The text before and after the caret.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.caret)
    }

    /// Move the caret to the character boundary nearest `x`, a distance
    /// from the start of the text, where `advance` gives the width of
    /// each text that starts the line. A tie goes to the earlier
    /// boundary. With `extend`, the selection runs from the anchor to
    /// the new caret; without it, nothing is selected.
    pub fn place(&mut self, x: f32, extend: bool, advance: impl Fn(&str) -> f32) {
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
        self.move_to(best.0, extend);
    }

    /// Move the caret to `to`, keeping the anchor with `extend` and
    /// dropping the selection without it.
    fn move_to(&mut self, to: usize, extend: bool) {
        self.caret = to;
        if !extend {
            self.anchor = to;
        }
    }

    /// Remove the selection, or with nothing selected the text between
    /// the caret and `to`, on either side of it. Returns whether
    /// anything was removed.
    fn delete_to(&mut self, to: usize) -> bool {
        let range = match self.selection() {
            sel if sel.is_empty() => self.caret.min(to)..self.caret.max(to),
            sel => sel,
        };
        self.replace(range, "")
    }

    /// Replace the selection, or insert at the caret with nothing
    /// selected, with `text`, and move the caret past it. Returns
    /// whether anything changed; inserting nothing changes nothing.
    fn insert(&mut self, text: &str) -> bool {
        !text.is_empty() && self.replace(self.selection(), text)
    }

    /// Replace `range` with `text`, the caret after it and nothing
    /// selected. Returns whether the text changed.
    fn replace(&mut self, range: Range<usize>, text: &str) -> bool {
        let changed = self.text[range.clone()] != *text;
        let at = range.start + text.len();
        self.text.replace_range(range, text);
        self.caret = at;
        self.anchor = at;
        changed
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
    /// remove, no room to move, an empty paste, a copy with nothing
    /// selected.
    Ignored,
    /// The caret or the selection moved; the text is unchanged.
    Moved,
    /// The text changed.
    Edited,
    /// The selection went to the clipboard; nothing changed.
    Copied,
}

/// Apply a key press to a one-line text entry at its caret.
///
/// Left and Right move the caret a character, with Ctrl or Alt a word,
/// and with Cmd to the start or end; Home and End move it to the start
/// or end. With Shift each of them moves the caret and keeps the
/// anchor, selecting the text between; without Shift it drops the
/// selection, and a plain Left or Right with text selected moves the
/// caret to the start or end of the selection. Ctrl+A or Cmd+A selects
/// all of the text.
///
/// Backspace removes the character before the caret, with Ctrl or Alt
/// the word before it, and with Cmd everything before it; Delete
/// removes the character after the caret, with Ctrl or Alt the word
/// after it. With text selected, either removes the selection. Ctrl+V
/// or Cmd+V inserts the text `paste` returns as one line, and a key
/// that types a character without Ctrl or Cmd inserts it; either
/// replaces the selection. Ctrl+C or Cmd+C passes the selection to
/// `copy`, and Ctrl+X or Cmd+X passes it and removes it.
///
/// The caller restarts its caret on a move or edit, and clears a
/// rejection on an edit.
pub fn edit_line(
    line: &mut Line,
    keystroke: &Keystroke,
    paste: impl FnOnce() -> Option<String>,
    copy: impl FnOnce(&str),
) -> LineKey {
    let mods = &keystroke.modifiers;
    let meta = mods.control || mods.platform;
    let word = mods.control || mods.alt;
    let key = keystroke.key.as_str();
    let sel = line.selection();
    let collapse = !sel.is_empty() && !mods.shift && !word && !mods.platform;
    let to = match key {
        "left" if collapse => Some(sel.start),
        "right" if collapse => Some(sel.end),
        "left" if mods.platform => Some(0),
        "left" if word => Some(line.word_start()),
        "left" => Some(line.prev_char()),
        "right" if mods.platform => Some(line.text.len()),
        "right" if word => Some(line.word_end()),
        "right" => Some(line.next_char()),
        "home" => Some(0),
        "end" => Some(line.text.len()),
        _ => None,
    };
    let was = (line.caret, line.anchor);
    let mut copied = false;
    let edited = match (to, key) {
        (Some(to), _) => {
            line.move_to(to, mods.shift);
            false
        }
        (None, "backspace") if mods.platform => line.delete_to(0),
        (None, "backspace") if word => line.delete_to(line.word_start()),
        (None, "backspace") => line.delete_to(line.prev_char()),
        (None, "delete") if word => line.delete_to(line.word_end()),
        (None, "delete") => line.delete_to(line.next_char()),
        (None, "a") if meta => {
            line.anchor = 0;
            line.caret = line.text.len();
            false
        }
        (None, "c" | "x") if meta && !sel.is_empty() => {
            copy(&line.text[sel.clone()]);
            copied = true;
            key == "x" && line.replace(sel, "")
        }
        (None, "v") if meta => paste().is_some_and(|text| line.insert(&one_line(&text))),
        _ => match (&keystroke.key_char, meta) {
            (Some(ch), false) => line.insert(ch),
            _ => false,
        },
    };
    if edited {
        LineKey::Edited
    } else if (line.caret, line.anchor) != was {
        LineKey::Moved
    } else if copied {
        LineKey::Copied
    } else {
        LineKey::Ignored
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

    use super::{edit_line, keep_in_view, ring_step, Line, LineKey};

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

    /// The line `marked` writes, its caret at the `|` and its anchor at
    /// the `^`, or at the caret without one.
    fn line(marked: &str) -> Line {
        let at = |mark: char| {
            let i = marked.find(mark)?;
            Some(i - marked[..i].matches(['|', '^']).count())
        };
        let caret = at('|').expect("the marked line has a caret");
        let mut line = Line::new(marked.replace(['|', '^'], ""));
        line.caret = caret;
        line.anchor = at('^').unwrap_or(caret);
        line
    }

    /// `line` written with a `|` at its caret and a `^` at its anchor
    /// when text is selected.
    fn marked(line: &Line) -> String {
        let mut text = line.text().to_string();
        let mut marks = [(line.caret(), '|'), (line.anchor(), '^')];
        marks.sort_by_key(|&(at, _)| std::cmp::Reverse(at));
        for (at, mark) in marks {
            if mark == '|' || at != line.caret() {
                text.insert(at, mark);
            }
        }
        text
    }

    /// The marked line after `start` takes `keystroke` with `clipboard`
    /// on the clipboard, what the key did, and what it copied.
    fn after_copy(
        start: &str,
        keystroke: &Keystroke,
        clipboard: Option<&str>,
    ) -> (String, LineKey, Option<String>) {
        let mut line = line(start);
        let mut copied = None;
        let key = edit_line(
            &mut line,
            keystroke,
            || clipboard.map(Into::into),
            |s| copied = Some(s.to_string()),
        );
        (marked(&line), key, copied)
    }

    /// The marked line after `start` takes `keystroke` with `clipboard`
    /// on the clipboard, and what the key did.
    fn after(start: &str, keystroke: &Keystroke, clipboard: Option<&str>) -> (String, LineKey) {
        let (end, key, _) = after_copy(start, keystroke, clipboard);
        (end, key)
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
                "^ab|",
                LineKey::Moved,
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
    /// Shift moves it as the key alone does and selects what it passes.
    /// None of them changes the text, and a move with no room is
    /// ignored.
    #[test]
    fn arrows_home_and_end_move_the_caret_without_editing() {
        let (left, right) = (
            |mods| key("left", None, mods),
            |mods| key("right", None, mods),
        );
        check(&[
            ("ab|c", left(""), "a|bc", LineKey::Moved),
            ("ab|c", left("shift"), "a|b^c", LineKey::Moved),
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
            ("ab|c", key("end", None, "shift"), "ab^c|", LineKey::Moved),
            ("|abc", key("home", None, ""), "|abc", LineKey::Ignored),
            ("abc|", key("end", None, ""), "abc|", LineKey::Ignored),
        ]);
    }

    // WHY: closes "a key lands the caret or the anchor between the
    // bytes of a character or reports the wrong outcome" for every key
    // the entry takes, with every modifier, from every caret and anchor
    // position of text with one-, two-, three-, and four-byte
    // characters. Both stay on character boundaries of the text, a move
    // leaves the text and changes the caret or the selection, an edit
    // changes the text, a copy changes nothing, an ignored key changes
    // nothing, and only a copy or a cut reaches the clipboard, with the
    // selected text, so the caller's caret restart, rejection reset,
    // and Copy fallback follow what happened.
    #[test]
    fn every_key_leaves_the_caret_and_anchor_on_character_boundaries() {
        let keys = [
            "left",
            "right",
            "home",
            "end",
            "backspace",
            "delete",
            "a",
            "c",
            "v",
            "x",
        ];
        let mods = ["", "shift", "ctrl", "alt", "cmd", "ctrl shift", "alt shift"];
        for text in ["", "a", "ab cd", "naïve—café ✓ 𝄞x", "--_{date}"] {
            let bounds = || text.char_indices().map(|(i, _)| i).chain([text.len()]);
            for (caret, anchor) in bounds().flat_map(|c| bounds().map(move |a| (c, a))) {
                for name in keys {
                    for mods in mods {
                        let mut line = Line::new(text);
                        (line.caret, line.anchor) = (caret, anchor);
                        let selected = &text[caret.min(anchor)..caret.max(anchor)];
                        let typed = match name {
                            "x" => Some("é"),
                            "a" | "c" | "v" => Some(name),
                            _ => None,
                        };
                        let keystroke = key(name, typed, mods);
                        let mut copied = None;
                        let outcome = edit_line(
                            &mut line,
                            &keystroke,
                            || Some("ü\n".into()),
                            |s| copied = Some(s.to_string()),
                        );
                        let at = format!("{text:?} at {caret}^{anchor} + {mods} {name}");
                        for offset in [line.caret(), line.anchor()] {
                            assert!(
                                line.text().is_char_boundary(offset),
                                "{at}: {offset} is inside a character"
                            );
                        }
                        let now = (line.text(), line.caret(), line.anchor());
                        let cut = (name == "x" && outcome == LineKey::Edited).then_some(selected);
                        match outcome {
                            LineKey::Ignored | LineKey::Copied => {
                                assert_eq!(now, (text, caret, anchor), "{at}")
                            }
                            LineKey::Moved => {
                                assert_eq!(line.text(), text, "{at}");
                                assert_ne!((now.1, now.2), (caret, anchor), "{at}");
                            }
                            LineKey::Edited => assert_ne!(line.text(), text, "{at}"),
                        }
                        let expected = match outcome {
                            LineKey::Copied => Some(selected),
                            _ if !mods.contains("ctrl") && !mods.contains("cmd") => None,
                            _ => cut,
                        };
                        assert_eq!(copied.as_deref(), expected, "{at}: clipboard");
                        if copied.is_some() {
                            assert!(!selected.is_empty(), "{at}: copied nothing");
                        }
                    }
                }
            }
        }
    }

    // WHY: closes "a selection is ignored, survives a key that ends it,
    // or is removed by a key that should replace it": Shift with each
    // move extends it from the anchor, a plain Left or Right collapses
    // it to its start or end, every other move drops it, Backspace and
    // Delete with any modifier remove it, typing and a paste replace
    // it, Ctrl+A and Cmd+A select everything, and Ctrl+C, Cmd+C,
    // Ctrl+X, and Cmd+X copy it, the cut also removing it. It does not
    // cover drawing the selection; the X11 tests read it on screen.
    #[test]
    fn a_selection_extends_collapses_and_is_replaced_removed_and_copied() {
        let (left, right) = (
            |mods| key("left", None, mods),
            |mods| key("right", None, mods),
        );
        let typed = |c| key(c, Some(c), "");
        check(&[
            ("ab|cd", left("shift"), "a|b^cd", LineKey::Moved),
            ("a|b^cd", left("shift"), "|ab^cd", LineKey::Moved),
            ("a|b^cd", right("shift"), "ab|cd", LineKey::Moved),
            ("|ab^cd", left("shift"), "|ab^cd", LineKey::Ignored),
            ("|ab cd", right("ctrl shift"), "^ab| cd", LineKey::Moved),
            ("ab |cd", right("cmd shift"), "ab ^cd|", LineKey::Moved),
            (
                "ab| cd",
                key("home", None, "shift"),
                "|ab^ cd",
                LineKey::Moved,
            ),
            (
                "ab |cd",
                key("end", None, "shift"),
                "ab ^cd|",
                LineKey::Moved,
            ),
            ("a^bc|d", left(""), "a|bcd", LineKey::Moved),
            ("a|bc^d", right(""), "abc|d", LineKey::Moved),
            ("a^bc|d", left("ctrl"), "|abcd", LineKey::Moved),
            ("a^bc|d", right("cmd"), "abcd|", LineKey::Moved),
            ("a^bc|d", key("home", None, ""), "|abcd", LineKey::Moved),
            ("a|bc^d", key("end", None, ""), "abcd|", LineKey::Moved),
            ("a^bc|d", key("backspace", None, ""), "a|d", LineKey::Edited),
            (
                "a^bc|d",
                key("backspace", None, "cmd"),
                "a|d",
                LineKey::Edited,
            ),
            ("a|bc^d", key("delete", None, ""), "a|d", LineKey::Edited),
            (
                "ab ^cd|",
                key("backspace", None, "ctrl"),
                "ab |",
                LineKey::Edited,
            ),
            ("a|bc^d", key("delete", None, "alt"), "a|d", LineKey::Edited),
            ("a^bc|d", typed("x"), "ax|d", LineKey::Edited),
            ("a^b|", typed("b"), "ab|", LineKey::Moved),
            ("a^bc|d", key("up", None, ""), "a^bc|d", LineKey::Ignored),
            ("ab|", key("a", Some("a"), "ctrl"), "^ab|", LineKey::Moved),
            ("a|b", key("a", Some("a"), "cmd"), "^ab|", LineKey::Moved),
            (
                "^ab|",
                key("a", Some("a"), "ctrl"),
                "^ab|",
                LineKey::Ignored,
            ),
            ("|", key("a", Some("a"), "cmd"), "|", LineKey::Ignored),
        ]);
        for (start, keystroke, clipboard, end, outcome, copied) in [
            (
                "a^bc|d",
                key("c", Some("c"), "ctrl"),
                None,
                "a^bc|d",
                LineKey::Copied,
                Some("bc"),
            ),
            (
                "a|bc^d",
                key("c", Some("c"), "cmd"),
                None,
                "a|bc^d",
                LineKey::Copied,
                Some("bc"),
            ),
            (
                "a|bc^d",
                key("x", Some("x"), "cmd"),
                None,
                "a|d",
                LineKey::Edited,
                Some("bc"),
            ),
            (
                "^café|",
                key("x", Some("x"), "ctrl"),
                None,
                "|",
                LineKey::Edited,
                Some("café"),
            ),
            (
                "ab|",
                key("c", Some("c"), "ctrl"),
                None,
                "ab|",
                LineKey::Ignored,
                None,
            ),
            (
                "ab|",
                key("x", Some("x"), "ctrl"),
                None,
                "ab|",
                LineKey::Ignored,
                None,
            ),
            (
                "a^bc|d",
                key("v", Some("v"), "ctrl"),
                Some("Z\n"),
                "aZ|d",
                LineKey::Edited,
                None,
            ),
            (
                "a^bc|d",
                key("v", Some("v"), "ctrl"),
                Some(""),
                "a^bc|d",
                LineKey::Ignored,
                None,
            ),
        ] {
            assert_eq!(
                after_copy(start, &keystroke, clipboard),
                (end.to_string(), outcome, copied.map(Into::into)),
                "{start:?} + {keystroke:?} with {clipboard:?}"
            );
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
            placed.place(x, false, advance);
            assert_eq!(marked(&placed), end, "{start:?} at {x}");
        }
        // A Shift-click keeps the anchor and selects to the click; a
        // plain click drops the selection.
        for (start, x, extend, end) in [
            ("a|bcd", 30.0, true, "a^bc|d"),
            ("a^bc|d", 0.0, true, "|a^bcd"),
            ("a^bc|d", 10.0, true, "a|bcd"),
            ("a^bc|d", 40.0, false, "abcd|"),
        ] {
            let mut placed = line(start);
            placed.place(x, extend, |s| 10.0 * s.chars().count() as f32);
            assert_eq!(marked(&placed), end, "{start:?} at {x}");
        }
    }

    // WHY: closes "the caret of a long field value is drawn outside the
    // field" and its siblings: a view that jumps while the caret stays
    // in it, that runs past the text's end, or that scrolls a text that
    // fits. Over a grid of texts, carets, earlier scrolls, and views, the
    // caret is always in view, the scroll stays between the text's
    // start and the point where the view ends at the text's end, a
    // caret already in view leaves the scroll, and a caret out of view
    // moves it only as far as needed. It does not cover the shaped
    // widths the field measures with; settings_paste.rs reads the
    // caret on screen.
    #[test]
    fn a_long_text_scrolls_only_as_far_as_the_caret_leaves_the_view() {
        let steps = |n: u16, step: f32| (0..=n).map(move |i| f32::from(i) * step);
        for view in [10.0, 46.0, 182.0] {
            for width in steps(30, 10.0) {
                let most = (width - view).max(0.0);
                for caret in steps(30, 10.0).filter(|&c| c <= width) {
                    for was in steps(30, 10.0).map(|w| w.min(most)) {
                        let s = keep_in_view(was, caret, width, view);
                        let at = format!("caret {caret} of {width} in {view} from {was}");
                        assert!((0.0..=most).contains(&s), "{at}: scroll {s}");
                        assert!(s <= caret && caret <= s + view, "{at}: scroll {s}");
                        let expected = if caret < was {
                            caret
                        } else if caret > was + view {
                            caret - view
                        } else {
                            was
                        };
                        assert_eq!(s, expected, "{at}");
                    }
                }
            }
        }
        // A field opened on a text it fits never scrolls; one opened
        // with the caret after a longer text shows the text's end.
        assert_eq!(keep_in_view(0.0, 120.0, 120.0, 182.0), 0.0);
        assert_eq!(keep_in_view(0.0, 300.0, 300.0, 182.0), 118.0);
        // A text that shrinks under the scroll pulls it back.
        assert_eq!(keep_in_view(118.0, 150.0, 200.0, 182.0), 18.0);
    }
}
