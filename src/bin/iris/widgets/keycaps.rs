//! A key chord drawn as key caps: "Ctrl+Shift+R" as three small
//! rounded chips, the way a shortcut reads on a keyboard.

use gpui::*;

use crate::theme;

/// Cap height and corner radius, logical px.
const CAP_H: f32 = 18.0;
const CAP_RADIUS: f32 = 4.0;

/// The keys of `chord` in display case, in the order written: known
/// modifiers and named keys by their keyboard label, a single character
/// upper case, a function key as `F` and its number. Empty segments
/// (a stray `+`) are dropped.
pub fn chord_keys(chord: &str) -> Vec<SharedString> {
    chord
        .split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(key_label)
        .collect()
}

/// `chord` written in display case, its keys joined by `+`: the label a
/// sentence or a tooltip shows for it.
pub fn chord_label(chord: &str) -> String {
    chord_keys(chord).join("+")
}

fn key_label(key: &str) -> SharedString {
    match named(key) {
        Some(label) => label.into(),
        None => capitalized(key).into(),
    }
}

/// The keyboard label of a modifier or named key, in any case and with
/// or without spaces ("Page Up", "pgup"); None for any other key.
fn named(key: &str) -> Option<&'static str> {
    Some(match key.to_lowercase().replace(' ', "").as_str() {
        "ctrl" | "control" => "Ctrl",
        "shift" => "Shift",
        "alt" => "Alt",
        "option" => "Option",
        "super" => "Super",
        "cmd" | "command" => "Cmd",
        "meta" => "Meta",
        "win" | "windows" => "Win",
        "print" | "printscreen" | "prtscn" | "prtsc" | "prtscr" => "Print",
        "escape" | "esc" => "Esc",
        "enter" | "return" => "Enter",
        "space" | "spacebar" => "Space",
        "tab" => "Tab",
        "backspace" => "Backspace",
        "delete" | "del" => "Delete",
        "home" => "Home",
        "end" => "End",
        "pageup" | "pgup" => "Page Up",
        "pagedown" | "pgdn" => "Page Down",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "arrow" => "Arrow",
        "arrows" => "Arrows",
        _ => return None,
    })
}

/// `chord` as a row of key caps in secondary text: a faint fill and no
/// outline, so a shortcut reads as a hint beside its label rather than
/// as a row of buttons. Tabular figures keep a function key's number
/// from shifting the row.
pub fn keycaps(chord: &str) -> Div {
    keycaps_in(chord, theme::FG_DIM)
}

/// `chord` as [`keycaps`] with its labels in `ink`: primary text where
/// the chord is a control's value, as in a shortcut field.
pub fn keycaps_in(chord: &str, ink: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(3.))
        .children(chord_keys(chord).into_iter().map(|key| cap(key, ink)))
}

fn cap(key: SharedString, ink: Rgba) -> Div {
    div()
        .h(px(CAP_H))
        .min_w(px(CAP_H))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(CAP_RADIUS))
        .bg(theme::alpha(theme::FG, 0.08))
        .text_size(px(theme::TEXT_SMALL))
        .line_height(px(14.))
        .font_weight(FontWeight::MEDIUM)
        .font_features(theme::tabular())
        .text_color(ink)
        .whitespace_nowrap()
        .child(key)
}

/// One piece of a shortcut hint: a key, drawn as a cap, or a word of
/// the instruction around the keys, drawn as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Piece {
    Cap(SharedString),
    Word(SharedString),
}

/// The separators of a hint's alternatives and steps, longest first so
/// `, then ` wins over `, `, each with the word it shows as.
const JOINERS: [(&str, &str); 5] = [
    (", then ", "then"),
    (" / ", "/"),
    (" or ", "or"),
    (" + ", "+"),
    (", ", ","),
];

/// Whether `word` names a key: a modifier or named key in any spelling
/// `chord_keys` reads, a function key, or a single character.
fn is_key(word: &str) -> bool {
    let mut chars = word.chars();
    let single = chars.next().is_some() && chars.next().is_none();
    let function = word
        .strip_prefix(['f', 'F'])
        .is_some_and(|n| matches!(n.parse::<u8>(), Ok(1..=24)));
    single || function || named(word).is_some()
}

/// `word` with its first character upper case: how a key without a
/// name reads.
fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The pieces of a shortcut hint, in reading order: "Shift+F10, then
/// Left / Right" reads as Shift, F10, then, Left, /, Right. A chord's
/// keys are caps and its other parts words ("Shift+click"); a phrase
/// splits into words ("Esc twice"), except a key name of two words
/// ("Page Up").
pub(crate) fn hint_pieces(hint: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = hint;
    loop {
        let next = (0..rest.len())
            .filter(|&i| rest.is_char_boundary(i))
            .find_map(|i| {
                JOINERS
                    .iter()
                    .find(|(sep, _)| rest[i..].starts_with(sep))
                    .map(|&(sep, word)| (i, sep, word))
            });
        let Some((at, sep, word)) = next else {
            segment(rest, &mut out);
            return out;
        };
        segment(&rest[..at], &mut out);
        out.push(Piece::Word(word.into()));
        rest = &rest[at + sep.len()..];
    }
}

fn segment(seg: &str, out: &mut Vec<Piece>) {
    let seg = seg.trim();
    if is_key(seg) {
        out.push(Piece::Cap(key_label(seg)));
    } else if seg.contains(' ') {
        for word in seg.split_whitespace() {
            segment(word, out);
        }
    } else if seg.contains('+') {
        for part in seg.split('+').filter(|p| !p.is_empty()) {
            segment(part, out);
        }
    } else if !seg.is_empty() {
        out.push(Piece::Word(seg.to_string().into()));
    }
}

/// A shortcut hint as a row of key caps and the words between them, in
/// `ink`: the keys of a shortcuts sheet row.
pub fn key_hint(hint: &str, ink: Rgba) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(3.))
        .children(hint_pieces(hint).into_iter().map(|piece| {
            match piece {
                Piece::Cap(key) => cap(key, ink),
                Piece::Word(word) => div()
                    .px(px(2.))
                    .text_size(px(theme::TEXT_SMALL))
                    .text_color(ink)
                    .whitespace_nowrap()
                    .child(word),
            }
        }))
}

/// `text` around its first whole word `key`, the sides trimmed; None
/// when `key` shows only inside a longer word or not at all.
fn split_at_key<'a>(text: &'a str, key: &str) -> Option<(&'a str, &'a str)> {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    text.match_indices(key).find_map(|(at, _)| {
        let (before, after) = (&text[..at], &text[at + key.len()..]);
        (!word(before.chars().next_back()) && !word(after.chars().next()))
            .then(|| (before.trim_end(), after.trim_start()))
    })
}

/// `text` in `ink` with its word `key` drawn as a cap: a status line
/// that names the key to press. Without the word, plain text.
pub fn text_with_key(text: &str, key: &str, ink: Rgba) -> Div {
    let Some((before, after)) = split_at_key(text, key) else {
        return div().text_color(ink).child(text.to_string());
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(4.))
        .text_color(ink)
        .children((!before.is_empty()).then(|| before.to_string()))
        .child(cap(key_label(key), ink))
        .children((!after.is_empty()).then(|| after.to_string()))
}

/// The words a shortcut hint may show beside its keys. A sheet row with
/// any other word fails `assert_hints_read_as_keys`: a key the grammar
/// misreads as a word ("Page Up/Down") would show as text.
#[cfg(test)]
const INSTRUCTION_WORDS: [&str; 5] = ["click", "right-click", "Double-click", "drag", "twice"];

/// Assert that each sheet row's keys read as caps, with only joiners
/// and instruction words as text.
#[cfg(test)]
pub(crate) fn assert_hints_read_as_keys(rows: &[(&'static str, SharedString)]) {
    for (action, hint) in rows {
        for piece in hint_pieces(hint) {
            if let Piece::Word(word) = piece {
                assert!(
                    JOINERS.iter().any(|(_, j)| *j == word.as_ref())
                        || INSTRUCTION_WORDS.contains(&word.as_ref()),
                    "{action}: {hint:?} shows {word:?} as text, not a key cap"
                );
            }
        }
    }
}

// WHY: the class closed here is "a cap shows a key other than the one
// the chord binds, or a blank cap", and for a sheet hint, "a key shows
// as text or a word as a cap". The config grammar accepts any case and
// several spellings per key; the caps read one label per key and drop
// the empty segments of a stray `+`. A hint splits at each joiner, a
// chord at `+`, and a phrase at spaces, except a two-word key name.
// Not covered: the rendered caps, checked on docshots.
#[cfg(test)]
mod tests {
    use std::prelude::v1::test;

    use super::*;

    #[test]
    fn chords_split_into_display_labels() {
        for (chord, want) in [
            ("Ctrl+Shift+R", vec!["Ctrl", "Shift", "R"]),
            ("ctrl+shift+r", vec!["Ctrl", "Shift", "R"]),
            ("CONTROL + prtsc", vec!["Ctrl", "Print"]),
            ("Print", vec!["Print"]),
            ("f9", vec!["F9"]),
            ("Super+pgdn", vec!["Super", "Page Down"]),
            ("Cmd+Option+5", vec!["Cmd", "Option", "5"]),
            ("Ctrl++L", vec!["Ctrl", "L"]),
            ("  ", vec![]),
        ] {
            let got: Vec<SharedString> = chord_keys(chord);
            let got: Vec<&str> = got.iter().map(|s| s.as_ref()).collect();
            assert_eq!(got, want, "{chord:?}");
        }
    }

    #[test]
    fn a_chord_label_joins_its_display_keys() {
        assert_eq!(chord_label(" ctrl + shift+r"), "Ctrl+Shift+R");
        assert_eq!(chord_label("prtsc"), "Print");
        assert_eq!(chord_label(""), "");
    }

    #[test]
    fn a_hint_reads_its_keys_as_caps_and_the_rest_as_words() {
        // `K` a cap, `W` a word.
        const K: bool = true;
        const W: bool = false;
        for (hint, want) in [
            (
                "Shift+F10, then Left / Right",
                vec![
                    (K, "Shift"),
                    (K, "F10"),
                    (W, "then"),
                    (K, "Left"),
                    (W, "/"),
                    (K, "Right"),
                ],
            ),
            (
                "Ctrl+S / Cmd+W / return",
                vec![
                    (K, "Ctrl"),
                    (K, "S"),
                    (W, "/"),
                    (K, "Cmd"),
                    (K, "W"),
                    (W, "/"),
                    (K, "Enter"),
                ],
            ),
            (
                "0 / + / -",
                vec![(K, "0"), (W, "/"), (K, "+"), (W, "/"), (K, "-")],
            ),
            ("Space + drag", vec![(K, "Space"), (W, "+"), (W, "drag")]),
            ("Esc twice", vec![(K, "Esc"), (W, "twice")]),
            (
                "Shift+F10 or right-click",
                vec![(K, "Shift"), (K, "F10"), (W, "or"), (W, "right-click")],
            ),
            ("Ctrl+click", vec![(K, "Ctrl"), (W, "click")]),
            (
                "Arrows, Home, Page Up",
                vec![
                    (K, "Arrows"),
                    (W, ","),
                    (K, "Home"),
                    (W, ","),
                    (K, "Page Up"),
                ],
            ),
            (
                "Tab, then Space",
                vec![(K, "Tab"), (W, "then"), (K, "Space")],
            ),
            ("Cmd+,", vec![(K, "Cmd"), (K, ",")]),
            ("?", vec![(K, "?")]),
            ("Double-click", vec![(W, "Double-click")]),
            ("", vec![]),
        ] {
            let got: Vec<(bool, String)> = hint_pieces(hint)
                .into_iter()
                .map(|p| match p {
                    Piece::Cap(k) => (K, k.to_string()),
                    Piece::Word(w) => (W, w.to_string()),
                })
                .collect();
            let want: Vec<(bool, String)> =
                want.into_iter().map(|(k, s)| (k, s.to_string())).collect();
            assert_eq!(got, want, "{hint:?}");
        }
    }

    #[test]
    fn a_key_splits_its_sentence_only_as_a_whole_word() {
        for (text, key, want) in [
            (
                "Press Esc again to discard changes",
                "Esc",
                Some(("Press", "again to discard changes")),
            ),
            ("Esc to close", "Esc", Some(("", "to close"))),
            ("then press Esc.", "Esc", Some(("then press", "."))),
            (
                "Escape twice, then Esc",
                "Esc",
                Some(("Escape twice, then", "")),
            ),
            ("Escape twice", "Esc", None),
            ("Press Tab", "Esc", None),
        ] {
            assert_eq!(split_at_key(text, key), want, "{text:?}");
        }
    }
}
