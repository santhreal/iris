//! What each Settings pane shows: its tab, its groups of rows, and the
//! window height that fits them. The render and the height both read
//! these tables, so a row added to a pane also grows its window.

use gpui::SharedString;
use iris_lib::config::{
    Config, RecordingEncoder, RecordingFormat, ToastClickAction, ToastPosition, UpdateChannel,
};

use super::Field;
use crate::icons::Icon;
use crate::theme;

/// The window's fixed width; the height follows the pane.
pub(super) const WIDTH: f32 = 600.0;
/// A group header: one caption line and the gap above its card.
const HEADER_LINE: f32 = 14.0;
pub(super) const HEADER_H: f32 = HEADER_LINE + theme::HEADER_GAP;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pane {
    General,
    Capture,
    Recording,
    Shortcuts,
    Updates,
}

/// A titled run of rows on one card.
pub(crate) struct Group {
    pub(super) header: Option<&'static str>,
    pub(super) rows: &'static [Row],
}

/// One row of a pane: a label on the left, its control on the right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Row {
    Switch(Switch),
    Pick(Pick),
    /// A typed value, committed on Return or when the field loses focus.
    Text(Field),
    /// A directory: the path and a Choose… button.
    Folder(Field),
    /// A key combination, recorded from the next key press.
    Hotkey(Field),
    /// Restore every setting to its default.
    Restore,
    /// The running version, the update state, and Check Now / Install.
    Version,
}

impl Row {
    pub(super) fn height(self) -> f32 {
        match self {
            Row::Version => theme::ROW_H_TALL,
            _ => theme::ROW_H,
        }
    }
}

impl Pane {
    pub(super) const ALL: [Pane; 5] = [
        Pane::General,
        Pane::Capture,
        Pane::Recording,
        Pane::Shortcuts,
        Pane::Updates,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Pane::General => "General",
            Pane::Capture => "Capture",
            Pane::Recording => "Recording",
            Pane::Shortcuts => "Shortcuts",
            Pane::Updates => "Updates",
        }
    }

    pub(super) fn icon(self) -> Icon {
        match self {
            Pane::General => Icon::Gear,
            Pane::Capture => Icon::Viewfinder,
            Pane::Recording => Icon::Record,
            Pane::Shortcuts => Icon::Keyboard,
            Pane::Updates => Icon::Download,
        }
    }

    pub(super) fn index(self) -> usize {
        Pane::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }

    /// The pane a Ctrl/Cmd+digit press selects: 1 is the first tab.
    pub(super) fn for_key(key: &str) -> Option<Pane> {
        let n: usize = key.parse().ok()?;
        Pane::ALL.get(n.checked_sub(1)?).copied()
    }

    pub(super) fn groups(self) -> &'static [Group] {
        match self {
            Pane::General => &[
                Group {
                    header: None,
                    rows: &[Row::Switch(Switch::Login)],
                },
                Group {
                    header: Some("Storage"),
                    rows: &[
                        Row::Folder(Field::ScreenshotsDir),
                        Row::Folder(Field::RecordingsDir),
                        Row::Text(Field::Template),
                    ],
                },
                Group {
                    header: None,
                    rows: &[Row::Restore],
                },
            ],
            Pane::Capture => &[
                Group {
                    header: None,
                    rows: &[
                        Row::Switch(Switch::Flash),
                        Row::Switch(Switch::Sound),
                        Row::Switch(Switch::Clipboard),
                    ],
                },
                Group {
                    header: Some("Floating Thumbnail"),
                    rows: &[
                        Row::Switch(Switch::Thumbnail),
                        Row::Pick(Pick::ToastClick),
                        Row::Pick(Pick::ToastDuration),
                        Row::Pick(Pick::ToastPosition),
                        Row::Switch(Switch::ToastDrag),
                        Row::Switch(Switch::ToastActions),
                    ],
                },
            ],
            Pane::Recording => &[Group {
                header: None,
                rows: &[
                    Row::Pick(Pick::RecordingFormat),
                    Row::Pick(Pick::RecordingEncoder),
                    Row::Text(Field::Fps),
                    Row::Switch(Switch::Mic),
                ],
            }],
            Pane::Shortcuts => &[
                Group {
                    header: Some("Global"),
                    rows: &[
                        Row::Hotkey(Field::CaptureHotkey),
                        Row::Hotkey(Field::RecordHotkey),
                    ],
                },
                Group {
                    header: Some("Selection Overlay"),
                    rows: &[
                        Row::Hotkey(Field::CancelKeybind),
                        Row::Hotkey(Field::ConfirmKeybind),
                    ],
                },
            ],
            Pane::Updates => &[
                Group {
                    header: None,
                    rows: &[Row::Switch(Switch::AutoCheck), Row::Pick(Pick::UpdateChannel)],
                },
                Group {
                    header: None,
                    rows: &[Row::Version],
                },
            ],
        }
    }

    /// The window height that shows every row of the pane: the toolbar,
    /// the content insets, and each group's header, rows, and the
    /// hairlines between them, with GROUP_GAP between groups.
    pub(super) fn height(self) -> f32 {
        let groups = self.groups();
        let body: f32 = groups
            .iter()
            .map(|g| {
                let header = if g.header.is_some() { HEADER_H } else { 0.0 };
                let rows: f32 = g.rows.iter().map(|r| r.height()).sum();
                header + rows + g.rows.len().saturating_sub(1) as f32
            })
            .sum();
        let gaps = groups.len().saturating_sub(1) as f32 * theme::GROUP_GAP;
        theme::TOOLBAR_H + 2.0 * theme::INSET + body + gaps
    }
}

/// An on/off setting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Switch {
    Login,
    Flash,
    Sound,
    Clipboard,
    Thumbnail,
    ToastDrag,
    ToastActions,
    Mic,
    AutoCheck,
}

impl Switch {
    #[cfg(test)]
    pub(super) const ALL: [Switch; 9] = [
        Switch::Login,
        Switch::Flash,
        Switch::Sound,
        Switch::Clipboard,
        Switch::Thumbnail,
        Switch::ToastDrag,
        Switch::ToastActions,
        Switch::Mic,
        Switch::AutoCheck,
    ];

    pub(super) fn id(self) -> &'static str {
        match self {
            Switch::Login => "tog-login",
            Switch::Flash => "tog-flash",
            Switch::Sound => "tog-sound",
            Switch::Clipboard => "tog-clipboard",
            Switch::Thumbnail => "tog-toast",
            Switch::ToastDrag => "tog-toast-drag",
            Switch::ToastActions => "tog-toast-actions",
            Switch::Mic => "tog-mic",
            Switch::AutoCheck => "tog-update-check",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Switch::Login => "Start iris at login",
            Switch::Flash => "Flash the screen",
            Switch::Sound => "Play shutter sound",
            Switch::Clipboard => "Copy to clipboard",
            Switch::Thumbnail => "Show floating thumbnail",
            Switch::ToastDrag => "Drag thumbnail to export",
            Switch::ToastActions => "Show action buttons",
            Switch::Mic => "Record microphone by default",
            Switch::AutoCheck => "Check for updates automatically",
        }
    }

    /// The config field the switch sets. Start at login is the login
    /// entry `sys::autostart` writes, not a config field: None.
    pub(super) fn slot(self, cfg: &mut Config) -> Option<&mut bool> {
        Some(match self {
            Switch::Login => return None,
            Switch::Flash => &mut cfg.flash_on_capture,
            Switch::Sound => &mut cfg.sound_on_capture,
            Switch::Clipboard => &mut cfg.copy_to_clipboard,
            Switch::Thumbnail => &mut cfg.show_toast_after_capture,
            Switch::ToastDrag => &mut cfg.toast_drag_enabled,
            Switch::ToastActions => &mut cfg.toast_show_actions,
            Switch::Mic => &mut cfg.record_mic_default,
            Switch::AutoCheck => &mut cfg.check_for_updates,
        })
    }
}

/// A choice among fixed options, shown as a pop-up button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pick {
    ToastClick,
    ToastDuration,
    ToastPosition,
    RecordingFormat,
    RecordingEncoder,
    UpdateChannel,
}

const TOAST_CLICKS: [ToastClickAction; 4] = [
    ToastClickAction::Markup,
    ToastClickAction::Copy,
    ToastClickAction::OpenFolder,
    ToastClickAction::None,
];
const TOAST_DURATIONS_MS: [u32; 5] = [2000, 3000, 5000, 8000, 10000];
const TOAST_POSITIONS: [ToastPosition; 4] = [
    ToastPosition::BottomRight,
    ToastPosition::BottomLeft,
    ToastPosition::TopRight,
    ToastPosition::TopLeft,
];
const FORMATS: [RecordingFormat; 3] = [
    RecordingFormat::Mp4,
    RecordingFormat::Gif,
    RecordingFormat::Webm,
];
const ENCODERS: [RecordingEncoder; 3] = [
    RecordingEncoder::Auto,
    RecordingEncoder::Libx264,
    RecordingEncoder::Nvenc,
];

impl Pick {
    #[cfg(test)]
    pub(super) const ALL: [Pick; 6] = [
        Pick::ToastClick,
        Pick::ToastDuration,
        Pick::ToastPosition,
        Pick::RecordingFormat,
        Pick::RecordingEncoder,
        Pick::UpdateChannel,
    ];

    pub(super) fn id(self) -> &'static str {
        match self {
            Pick::ToastClick => "pick-toast-click",
            Pick::ToastDuration => "pick-toast-duration",
            Pick::ToastPosition => "pick-toast-pos",
            Pick::RecordingFormat => "pick-rec-fmt",
            Pick::RecordingEncoder => "pick-rec-enc",
            Pick::UpdateChannel => "pick-update-channel",
        }
    }

    pub(super) fn menu_id(self) -> &'static str {
        match self {
            Pick::ToastClick => "menu-toast-click",
            Pick::ToastDuration => "menu-toast-duration",
            Pick::ToastPosition => "menu-toast-pos",
            Pick::RecordingFormat => "menu-rec-fmt",
            Pick::RecordingEncoder => "menu-rec-enc",
            Pick::UpdateChannel => "menu-update-channel",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Pick::ToastClick => "Click action",
            Pick::ToastDuration => "Dismiss after",
            Pick::ToastPosition => "Position",
            Pick::RecordingFormat => "Format",
            Pick::RecordingEncoder => "MP4 encoder",
            Pick::UpdateChannel => "Update channel",
        }
    }

    /// The option labels, in the order of the values they select.
    pub(super) fn options(self) -> &'static [&'static str] {
        match self {
            Pick::ToastClick => &["Open Editor", "Copy", "Show in Folder", "Nothing"],
            Pick::ToastDuration => &[
                "2 seconds",
                "3 seconds",
                "5 seconds",
                "8 seconds",
                "10 seconds",
            ],
            Pick::ToastPosition => &["Bottom Right", "Bottom Left", "Top Right", "Top Left"],
            Pick::RecordingFormat => &["MP4 (H.264)", "GIF", "WebM (VP9)"],
            Pick::RecordingEncoder => &["Automatic", "Software (x264)", "NVIDIA (NVENC)"],
            Pick::UpdateChannel => &["Stable", "Beta"],
        }
    }

    /// The option `cfg` holds, or None for a value no option lists (a
    /// toast duration written into config.toml by hand).
    pub(super) fn selected(self, cfg: &Config) -> Option<usize> {
        fn at<T: PartialEq>(all: &[T], v: &T) -> Option<usize> {
            all.iter().position(|x| x == v)
        }
        match self {
            Pick::ToastClick => at(&TOAST_CLICKS, &cfg.toast_click_action),
            Pick::ToastDuration => at(&TOAST_DURATIONS_MS, &cfg.toast_duration_ms),
            Pick::ToastPosition => at(&TOAST_POSITIONS, &cfg.toast_position),
            Pick::RecordingFormat => at(&FORMATS, &cfg.recording_format),
            Pick::RecordingEncoder => at(&ENCODERS, &cfg.recording_encoder),
            Pick::UpdateChannel => at(&UpdateChannel::ALL, &cfg.update_channel),
        }
    }

    /// Write option `ix` into `cfg`. An index past the options is
    /// ignored.
    pub(super) fn select(self, cfg: &mut Config, ix: usize) {
        match self {
            Pick::ToastClick => set(&mut cfg.toast_click_action, &TOAST_CLICKS, ix),
            Pick::ToastDuration => set(&mut cfg.toast_duration_ms, &TOAST_DURATIONS_MS, ix),
            Pick::ToastPosition => set(&mut cfg.toast_position, &TOAST_POSITIONS, ix),
            Pick::RecordingFormat => set(&mut cfg.recording_format, &FORMATS, ix),
            Pick::RecordingEncoder => set(&mut cfg.recording_encoder, &ENCODERS, ix),
            Pick::UpdateChannel => set(&mut cfg.update_channel, &UpdateChannel::ALL, ix),
        }
    }

    /// The pop-up button's text: the selected option, or the value
    /// itself when no option lists it.
    pub(super) fn current(self, cfg: &Config) -> SharedString {
        match self.selected(cfg) {
            Some(ix) => self.options()[ix].into(),
            None => match self {
                Pick::ToastDuration => seconds(cfg.toast_duration_ms).into(),
                _ => "".into(),
            },
        }
    }
}

fn set<T: Copy>(slot: &mut T, all: &[T], ix: usize) {
    if let Some(&v) = all.get(ix) {
        *slot = v;
    }
}

/// A duration in milliseconds as the duration pop-up words it.
fn seconds(ms: u32) -> String {
    if ms.is_multiple_of(1000) {
        let s = ms / 1000;
        format!("{s} second{}", if s == 1 { "" } else { "s" })
    } else {
        format!("{:.1} seconds", ms as f32 / 1000.0)
    }
}
