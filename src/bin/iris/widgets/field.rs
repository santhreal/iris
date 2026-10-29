//! Text fields and the folder field.

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
    /// Being edited: an accent focus ring and a caret after the text;
    /// `invalid` turns the ring destructive red.
    Editing { invalid: bool },
    /// Capturing a key combination: the focus ring, no caret, the text
    /// in secondary color (a prompt such as "Type shortcut").
    Recording,
}

/// The 3px focus ring around a control, drawn as a spread shadow so it
/// takes no layout space.
pub fn focus_ring(color: Rgba) -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: color.into(),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(3.),
        inset: false,
    }]
}

/// Single-line text field box, CONTROL_H tall: a dark fill with a 1px
/// inner border; editing adds the focus ring and the caret. The caller
/// has the text buffer and commits it (Return or blur).
pub fn text_field(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    state: FieldState,
) -> Stateful<Div> {
    let (border, ring) = match state {
        FieldState::Rest => (theme::FIELD_BORDER, None),
        FieldState::Editing { invalid: true } => {
            (theme::DANGER, Some(theme::alpha(theme::DANGER, 0.40)))
        }
        FieldState::Editing { invalid: false } | FieldState::Recording => {
            (theme::ACCENT, Some(theme::FOCUS_RING))
        }
    };
    let mut el = div()
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
        .cursor_text();
    if let Some(ring) = ring {
        el = el.shadow(focus_ring(ring));
    }
    el = el.child(text.into());
    if matches!(state, FieldState::Editing { .. }) {
        el = el.child(
            div()
                .w(px(1.5))
                .h(px(15.))
                .ml(px(1.))
                .flex_shrink_0()
                .bg(theme::ACCENT),
        );
    }
    el
}

/// A folder row control: a folder glyph and the path, middle-truncated
/// to `width` (the head and the folder name stay readable), then a
/// "Choose…" button that opens the platform's directory picker. A
/// chosen directory runs `on_pick(Ok(dir))`; a picker that fails to
/// open runs `on_pick(Err(message))`; a cancelled picker runs nothing.
pub fn folder_field(
    id: &'static str,
    shown: impl Into<SharedString>,
    width: f32,
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
        .child(
            push_button(id, "Choose\u{2026}", ButtonStyle::Bordered).on_click(
                move |_, window, cx| {
                    let rx = cx.prompt_for_paths(PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Choose".into()),
                    });
                    let on_pick = on_pick.clone();
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
                },
            ),
        )
}

/// Apply a key press to the buffer of a one-line text entry: Backspace
/// removes the last character, Ctrl+Backspace or Alt+Backspace the last
/// word (`pop_word`), and Cmd+Backspace the whole line; Ctrl+V or Cmd+V
/// appends the text `clipboard` returns as one line, and a key that
/// types a character without Ctrl or Cmd appends it. Returns whether
/// the key edited the buffer; the caller restarts its caret or clears a
/// rejection.
pub fn edit_line(
    buffer: &mut String,
    keystroke: &Keystroke,
    clipboard: impl FnOnce() -> Option<String>,
) -> bool {
    let mods = &keystroke.modifiers;
    let meta = mods.control || mods.platform;
    match keystroke.key.as_str() {
        "backspace" if mods.platform => {
            let edited = !buffer.is_empty();
            buffer.clear();
            edited
        }
        "backspace" if mods.control || mods.alt => pop_word(buffer),
        "backspace" => buffer.pop().is_some(),
        "v" if meta => clipboard().is_some_and(|text| push_one_line(buffer, &text)),
        _ => match (&keystroke.key_char, meta) {
            (Some(ch), false) => {
                buffer.push_str(ch);
                true
            }
            _ => false,
        },
    }
}

/// Remove the last word of `buffer` and what follows it: the trailing
/// run of characters that are not letters or digits, then the run of
/// letters and digits before it, as readline's `backward-kill-word`.
/// Returns whether anything was removed.
fn pop_word(buffer: &mut String) -> bool {
    let len = buffer.len();
    let word_end = buffer
        .trim_end_matches(|c: char| !c.is_alphanumeric())
        .len();
    let start = buffer[..word_end]
        .trim_end_matches(char::is_alphanumeric)
        .len();
    buffer.truncate(start);
    start < len
}

/// Append pasted `text` to `buffer` as one line: each line break or tab
/// is a space, a trailing line break is dropped, and other control
/// characters are removed. Returns whether anything was appended.
fn push_one_line(buffer: &mut String, text: &str) -> bool {
    let before = buffer.len();
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            buffer.push(' ');
        }
        buffer.extend(line.chars().filter_map(|c| match c {
            '\t' | '\r' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        }));
    }
    buffer.len() > before
}

#[cfg(test)]
mod tests {
    use gpui::{Keystroke, Modifiers};

    use super::edit_line;

    fn key(key: &str, key_char: Option<&str>, control: bool, platform: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                control,
                platform,
                ..Modifiers::default()
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    /// The buffer after `start` takes `keystroke` with `clipboard` on
    /// the clipboard, and whether the key edited it.
    fn after(start: &str, keystroke: &Keystroke, clipboard: Option<&str>) -> (String, bool) {
        let mut buffer = start.to_string();
        let edited = edit_line(&mut buffer, keystroke, || clipboard.map(Into::into));
        (buffer, edited)
    }

    /// A paste reaches a one-line entry as one line, from Ctrl+V and
    /// Cmd+V alike, whatever line breaks, tabs, or control characters the
    /// clipboard holds.
    #[test]
    fn a_paste_appends_the_clipboard_as_one_line() {
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
            for (control, platform) in [(true, false), (false, true)] {
                assert_eq!(
                    after("x ", &key("v", Some("v"), control, platform), Some(pasted)),
                    (format!("x {line}"), true),
                    "{pasted:?} with control {control} platform {platform}"
                );
            }
        }
    }

    /// Keys other than a paste type, delete, or do nothing; a paste of
    /// nothing edits nothing.
    #[test]
    fn typing_and_backspace_edit_and_chords_do_not() {
        let cases = [
            ("ab", key("c", Some("c"), false, false), None, "abc", true),
            (
                "ab",
                key("v", Some("v"), false, false),
                Some("zz"),
                "abv",
                true,
            ),
            ("ab", key("backspace", None, false, false), None, "a", true),
            ("", key("backspace", None, false, false), None, "", false),
            ("ab", key("c", Some("c"), true, false), None, "ab", false),
            ("ab", key("a", Some("a"), false, true), None, "ab", false),
            ("ab", key("v", Some("v"), true, false), None, "ab", false),
            (
                "ab",
                key("v", Some("v"), true, false),
                Some(""),
                "ab",
                false,
            ),
            (
                "ab",
                key("v", Some("v"), true, false),
                Some("\n"),
                "ab",
                false,
            ),
            ("ab", key("left", None, false, false), None, "ab", false),
        ];
        for (start, keystroke, clipboard, buffer, edited) in cases {
            assert_eq!(
                after(start, &keystroke, clipboard),
                (buffer.to_string(), edited),
                "{start:?} + {keystroke:?} with {clipboard:?}"
            );
        }
    }

    /// Backspace with Ctrl or Alt removes the last word and what follows
    /// it, a word being letters and digits; with Cmd it removes the line.
    /// A plain Backspace removes one character, whatever it is.
    #[test]
    fn a_modified_backspace_removes_the_last_word_or_the_line() {
        let backspace = |control, alt, platform| Keystroke {
            modifiers: Modifiers {
                control,
                alt,
                platform,
                ..Modifiers::default()
            },
            key: "backspace".into(),
            key_char: None,
        };
        let (plain, ctrl, alt, cmd) = (
            backspace(false, false, false),
            backspace(true, false, false),
            backspace(false, true, false),
            backspace(false, false, true),
        );
        let cases = [
            ("hello world", &ctrl, "hello ", true),
            ("hello world", &alt, "hello ", true),
            ("hello world  ", &ctrl, "hello ", true),
            ("~/Pictures/Screenshots/", &ctrl, "~/Pictures/", true),
            ("{date}_{time}", &alt, "{date}_{", true),
            ("naïve café", &ctrl, "naïve ", true),
            ("shot2024", &ctrl, "", true),
            ("--- ", &ctrl, "", true),
            ("", &ctrl, "", false),
            ("ab cd", &cmd, "", true),
            ("", &cmd, "", false),
            ("café", &plain, "caf", true),
        ];
        for (start, keystroke, buffer, edited) in cases {
            assert_eq!(
                after(start, keystroke, None),
                (buffer.to_string(), edited),
                "{start:?} + {:?}",
                keystroke.modifiers
            );
        }
    }
}
