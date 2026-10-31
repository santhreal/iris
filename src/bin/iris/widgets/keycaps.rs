//! A key chord drawn as key caps: "Ctrl+Shift+R" as three small
//! rounded chips, the way a shortcut reads on a keyboard.

use gpui::*;

use crate::theme;

/// Cap height and corner radius, logical px.
const CAP_H: f32 = 18.0;
const CAP_RADIUS: f32 = 4.0;

/// Whether shortcuts read as macOS writes them: modifiers and named keys
/// as their symbols, modifiers in the order Control, Option, Shift,
/// Command, and a chord's keys written with no separator.
const MAC: bool = cfg!(target_os = "macos");

/// The keys of `chord` in display case, in the order written: known
/// modifiers and named keys by their keyboard label, a single character
/// upper case, a function key as `F` and its number. Empty segments
/// (a stray `+`) are dropped. On macOS the modifiers lead in macOS's
/// order and read as symbols.
pub fn chord_keys(chord: &str) -> Vec<SharedString> {
    chord_keys_for(chord, MAC)
}

fn chord_keys_for(chord: &str, mac: bool) -> Vec<SharedString> {
    let mut keys: Vec<&str> = chord
        .split('+')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .collect();
    if mac {
        keys.sort_by_key(|key| modifier_rank(key));
    }
    keys.into_iter()
        .map(|key| key_label_for(key, mac))
        .collect()
}

/// `chord` in the config grammar, its keys in display case joined by
/// `+`: the form a hint is written in, on every platform.
pub fn chord_label(chord: &str) -> String {
    chord_keys_for(chord, false).join("+")
}

/// `hint`, a shortcut hint in the config grammar, as text on this
/// platform: what a tooltip or a sentence shows. Each chord reads as
/// its keys, joined by `+`, or on macOS as their symbols with no
/// separator ("Cmd+Shift+Z" reads "⇧⌘Z").
pub fn shortcut_text(hint: &str) -> String {
    hint_text_for(hint, MAC)
}

fn hint_text_for(hint: &str, mac: bool) -> String {
    let mut out = String::new();
    let mut rest = hint;
    loop {
        let Some((at, sep, _)) = next_joiner(rest) else {
            out.push_str(&segment_text(rest.trim(), mac));
            return out;
        };
        out.push_str(&segment_text(rest[..at].trim(), mac));
        out.push_str(sep);
        rest = &rest[at + sep.len()..];
    }
}

/// One segment of a hint between joiners, as text: a key or a chord by
/// its display keys, a phrase word by word.
fn segment_text(seg: &str, mac: bool) -> String {
    if is_key(seg) {
        return key_label_for(seg, mac).to_string();
    }
    if seg.contains(' ') {
        return seg
            .split_whitespace()
            .map(|word| segment_text(word, mac))
            .collect::<Vec<_>>()
            .join(" ");
    }
    if is_chord(seg) {
        return chord_keys_for(seg, mac).join(if mac { "" } else { "+" });
    }
    if mac && seg.contains('+') {
        // A modifier with a pointer action: "Command-click".
        return seg
            .split('+')
            .filter(|part| !part.is_empty())
            .map(|part| mac_name(part).unwrap_or(part))
            .collect::<Vec<_>>()
            .join("-");
    }
    seg.to_owned()
}

/// Whether `seg` is a chord: keys joined by `+`, every one a key.
fn is_chord(seg: &str) -> bool {
    seg.contains('+') && seg.split('+').filter(|p| !p.is_empty()).all(is_key)
}

fn key_label(key: &str) -> SharedString {
    key_label_for(key, MAC)
}

fn key_label_for(key: &str, mac: bool) -> SharedString {
    match named(key) {
        Some(label) => mac
            .then(|| mac_symbol(label))
            .flatten()
            .unwrap_or(label)
            .into(),
        None => capitalized(key).into(),
    }
}

/// The symbol macOS shows for the key labeled `label`, or None for a
/// key it writes as a word.
fn mac_symbol(label: &str) -> Option<&'static str> {
    Some(match label {
        "Ctrl" => "⌃",
        "Alt" | "Option" => "⌥",
        "Shift" => "⇧",
        "Cmd" | "Super" | "Meta" | "Win" => "⌘",
        "Enter" => "↩",
        "Tab" => "⇥",
        "Backspace" => "⌫",
        "Delete" => "⌦",
        "Up" => "↑",
        "Down" => "↓",
        "Left" => "←",
        "Right" => "→",
        "Page Up" => "⇞",
        "Page Down" => "⇟",
        "Home" => "↖",
        "End" => "↘",
        _ => return None,
    })
}

/// The word macOS writes for the modifier `key` in prose, None for any
/// other key.
fn mac_name(key: &str) -> Option<&'static str> {
    Some(match named(key)? {
        "Ctrl" => "Control",
        "Alt" | "Option" => "Option",
        "Shift" => "Shift",
        "Cmd" | "Super" | "Meta" | "Win" => "Command",
        _ => return None,
    })
}

/// Where `key` sits in a chord written as macOS writes it: Control,
/// Option, Shift, Command, then every other key.
fn modifier_rank(key: &str) -> u8 {
    match named(key) {
        Some("Ctrl") => 0,
        Some("Alt" | "Option") => 1,
        Some("Shift") => 2,
        Some("Cmd" | "Super" | "Meta" | "Win") => 3,
        _ => 4,
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

/// `chord` as a row of key caps with its labels in `ink`: a faint fill
/// and no outline, so a shortcut reads as a hint beside its label rather
/// than as a row of buttons. Tabular figures keep a function key's
/// number from shifting the row.
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

/// The first joiner in `rest`: its byte offset, its text, and the word
/// it shows as.
fn next_joiner(rest: &str) -> Option<(usize, &'static str, &'static str)> {
    (0..rest.len())
        .filter(|&i| rest.is_char_boundary(i))
        .find_map(|i| {
            JOINERS
                .iter()
                .find(|(sep, _)| rest[i..].starts_with(sep))
                .map(|&(sep, word)| (i, sep, word))
        })
}

/// The pieces of a shortcut hint, in reading order: "Shift+F10, then
/// Left / Right" reads as Shift, F10, then, Left, /, Right. A chord's
/// keys are caps, in macOS's order on macOS, and its other parts words
/// ("Shift+click"); a phrase splits into words ("Esc twice"), except a
/// key name of two words ("Page Up").
pub(crate) fn hint_pieces(hint: &str) -> Vec<Piece> {
    hint_pieces_for(hint, MAC)
}

fn hint_pieces_for(hint: &str, mac: bool) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = hint;
    loop {
        let Some((at, sep, word)) = next_joiner(rest) else {
            segment(rest, mac, &mut out);
            return out;
        };
        segment(&rest[..at], mac, &mut out);
        out.push(Piece::Word(word.into()));
        rest = &rest[at + sep.len()..];
    }
}

fn segment(seg: &str, mac: bool, out: &mut Vec<Piece>) {
    let seg = seg.trim();
    if is_key(seg) {
        out.push(Piece::Cap(key_label_for(seg, mac)));
    } else if seg.contains(' ') {
        for word in seg.split_whitespace() {
            segment(word, mac, out);
        }
    } else if is_chord(seg) {
        out.extend(chord_keys_for(seg, mac).into_iter().map(Piece::Cap));
    } else if seg.contains('+') {
        for part in seg.split('+').filter(|p| !p.is_empty()) {
            segment(part, mac, out);
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
        for (chord, pc, mac) in [
            (
                "Ctrl+Shift+R",
                vec!["Ctrl", "Shift", "R"],
                vec!["⌃", "⇧", "R"],
            ),
            (
                "ctrl+shift+r",
                vec!["Ctrl", "Shift", "R"],
                vec!["⌃", "⇧", "R"],
            ),
            ("CONTROL + prtsc", vec!["Ctrl", "Print"], vec!["⌃", "Print"]),
            ("Print", vec!["Print"], vec!["Print"]),
            ("f9", vec!["F9"], vec!["F9"]),
            ("Super+pgdn", vec!["Super", "Page Down"], vec!["⌘", "⇟"]),
            (
                "Cmd+Option+5",
                vec!["Cmd", "Option", "5"],
                vec!["⌥", "⌘", "5"],
            ),
            (
                "Cmd+Shift+Alt+Ctrl+enter",
                vec!["Cmd", "Shift", "Alt", "Ctrl", "Enter"],
                vec!["⌃", "⌥", "⇧", "⌘", "↩"],
            ),
            ("Win+Space", vec!["Win", "Space"], vec!["⌘", "Space"]),
            ("Ctrl++L", vec!["Ctrl", "L"], vec!["⌃", "L"]),
            ("  ", vec![], vec![]),
        ] {
            for (mac, want) in [(false, pc), (true, mac)] {
                let got: Vec<SharedString> = chord_keys_for(chord, mac);
                let got: Vec<&str> = got.iter().map(|s| s.as_ref()).collect();
                assert_eq!(got, want, "{chord:?} mac={mac}");
            }
        }
    }

    #[test]
    fn a_chord_label_joins_its_display_keys() {
        assert_eq!(chord_label(" ctrl + shift+r"), "Ctrl+Shift+R");
        assert_eq!(chord_label("cmd+option+shift+2"), "Cmd+Option+Shift+2");
        assert_eq!(chord_label("prtsc"), "Print");
        assert_eq!(chord_label(""), "");
    }

    // A hint in the config grammar reads unchanged off macOS, and on
    // macOS each chord reads as its symbols in macOS's order with no
    // separator, and a modifier with a pointer action as its name.
    #[test]
    fn a_hint_reads_as_text_in_the_platform_style() {
        for (hint, pc, mac) in [
            ("Ctrl+Shift+R", "Ctrl+Shift+R", "⌃⇧R"),
            ("ctrl+shift+r", "Ctrl+Shift+R", "⌃⇧R"),
            ("Cmd+Shift+Z", "Cmd+Shift+Z", "⇧⌘Z"),
            ("Cmd+Option+Shift+2", "Cmd+Option+Shift+2", "⌥⇧⌘2"),
            (
                "Shift+F10, then Left / Right",
                "Shift+F10, then Left / Right",
                "⇧F10, then ← / →",
            ),
            (
                "Ctrl+S / Cmd+W / Enter",
                "Ctrl+S / Cmd+W / Enter",
                "⌃S / ⌘W / ↩",
            ),
            (
                "Shift+F10 or right-click",
                "Shift+F10 or right-click",
                "⇧F10 or right-click",
            ),
            ("Cmd+click", "Cmd+click", "Command-click"),
            ("Space + drag", "Space + drag", "Space + drag"),
            ("Esc twice", "Esc twice", "Esc twice"),
            ("Print", "Print", "Print"),
            ("Cmd+,", "Cmd+,", "⌘,"),
            ("Double-click", "Double-click", "Double-click"),
            ("", "", ""),
        ] {
            assert_eq!(hint_text_for(hint, false), pc, "{hint:?}");
            assert_eq!(hint_text_for(hint, true), mac, "{hint:?} on macOS");
        }
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
            let got: Vec<(bool, String)> = hint_pieces_for(hint, false)
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

    // On macOS a chord's caps lead with its modifiers in macOS's order
    // and read as symbols; the words between keys stay words.
    #[test]
    fn on_macos_a_hint_reads_as_symbol_caps() {
        let caps = |hint: &str| -> Vec<String> {
            hint_pieces_for(hint, true)
                .into_iter()
                .map(|p| match p {
                    Piece::Cap(k) => format!("[{k}]"),
                    Piece::Word(w) => w.to_string(),
                })
                .collect()
        };
        assert_eq!(caps("Cmd+Shift+Z"), ["[⇧]", "[⌘]", "[Z]"]);
        assert_eq!(
            caps("Ctrl+S / Cmd+W / return"),
            ["[⌃]", "[S]", "/", "[⌘]", "[W]", "/", "[↩]"]
        );
        assert_eq!(caps("Shift+click"), ["[⇧]", "click"]);
        assert_eq!(caps("Esc twice"), ["[Esc]", "twice"]);
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
