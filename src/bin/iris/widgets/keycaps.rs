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
    let lower = key.to_lowercase();
    let named = match lower.as_str() {
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
        _ => {
            let mut chars = key.chars();
            return match chars.next() {
                Some(first) => {
                    let mut out: String = first.to_uppercase().collect();
                    out.push_str(chars.as_str());
                    out.into()
                }
                None => SharedString::default(),
            };
        }
    };
    named.into()
}

/// `chord` as a row of key caps in secondary text. Tabular figures keep
/// a function key's number from shifting the row.
pub fn keycaps(chord: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(3.))
        .children(chord_keys(chord).into_iter().map(|key| {
            div()
                .h(px(CAP_H))
                .min_w(px(CAP_H))
                .px(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(CAP_RADIUS))
                .bg(theme::alpha(theme::FG, 0.07))
                .border_1()
                .border_color(theme::HAIRLINE)
                .text_size(px(theme::TEXT_SMALL))
                .line_height(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .font_features(theme::tabular())
                .text_color(theme::FG_DIM)
                .whitespace_nowrap()
                .child(key)
        }))
}

// WHY: the class closed here is "a cap shows a key other than the one
// the chord binds, or a blank cap". The config grammar accepts any case
// and several spellings per key; the caps read one label per key and
// drop the empty segments of a stray `+`. Not covered: the rendered
// caps, checked on docshots.
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
}
