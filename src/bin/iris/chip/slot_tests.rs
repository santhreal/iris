//! WHY: the class closed here is "the timer slot names the wrong
//! control, or keeps a name after the pointer left". The slot shows the
//! hovered control's action for the chip's current state (Pause or
//! Resume, Mute or Unmute), the record hotkey only beside Stop, and the
//! clock again once the pointer leaves; a late leave event from the
//! previous control does not clear the next one's name. Every control
//! in every state names an action that fits the slot at its narrowest,
//! so a name never clips and the slot never grows under the pointer.
//! Not covered: GPUI's delivery of hover events to an unfocused popup.

use std::time::Instant;

use gpui::SharedString;

use super::{slot_w, Chip, Control, Slot};
use crate::theme;

/// Every control. The match fails to compile when a control is added,
/// so a new one cannot skip these cases.
fn all() -> [Control; 3] {
    match Control::Pause {
        Control::Pause | Control::Stop | Control::Mic => {}
    }
    [Control::Pause, Control::Stop, Control::Mic]
}

fn chip(mic: Option<bool>, keys: &str) -> Chip {
    let mut chip = Chip::new(mic, Instant::now());
    chip.record_hotkey = SharedString::from(keys.to_owned());
    chip
}

fn named(label: &'static str, keys: Option<&str>) -> Slot {
    Slot::Control {
        label,
        keys: keys.map(|k| SharedString::from(k.to_owned())),
    }
}

#[test]
fn each_control_names_its_action_in_the_current_state() {
    let mut c = chip(Some(true), "Ctrl+Shift+R");
    assert_eq!(c.slot(), Slot::Clock);

    c.hover(Control::Pause, true);
    assert_eq!(c.slot(), named("Pause", None));
    c.set_paused(true, Instant::now());
    assert_eq!(c.slot(), named("Resume", None), "the name follows a click");

    c.hover(Control::Stop, true);
    assert_eq!(c.slot(), named("Stop", Some("Ctrl+Shift+R")));

    c.hover(Control::Mic, true);
    assert_eq!(c.slot(), named("Mute", None));
    c.mic = Some(false);
    assert_eq!(c.slot(), named("Unmute", None));
}

#[test]
fn no_record_hotkey_means_no_shortcut_beside_stop() {
    let mut c = chip(None, "");
    c.hover(Control::Stop, true);
    assert_eq!(c.slot(), named("Stop", None));
}

#[test]
fn leaving_restores_the_clock_and_a_late_leave_changes_nothing() {
    let mut c = chip(Some(true), "Ctrl+Shift+R");
    c.hover(Control::Pause, true);
    // The pointer crosses from Pause to Stop; Stop's enter can arrive
    // before Pause's leave.
    c.hover(Control::Stop, true);
    c.hover(Control::Pause, false);
    assert_eq!(c.slot(), named("Stop", Some("Ctrl+Shift+R")));
    c.hover(Control::Stop, false);
    assert_eq!(c.slot(), Slot::Clock);
    // A leave with nothing hovered stays on the clock.
    c.hover(Control::Mic, false);
    assert_eq!(c.slot(), Slot::Clock);
}

#[test]
fn every_name_and_the_widest_clock_fit_the_narrowest_slot() {
    // The Regular face's advances, 10% over for the Medium weight the
    // slot draws and for Inter's kerning, which mostly tightens.
    let fits = |text: &str| theme::advance_sum(text, theme::TEXT_BODY) * 1.1 <= slot_w(true);
    assert!(fits("88:88"), "the clock in {}px", slot_w(true));
    for control in all() {
        for paused in [false, true] {
            for mic in [None, Some(false), Some(true)] {
                let label = control.label(paused, mic);
                assert!(
                    fits(label),
                    "{control:?} as {label:?} in {}px",
                    slot_w(true)
                );
            }
        }
    }
    assert!(
        slot_w(false) > slot_w(true),
        "no mic button leaves more room"
    );
}
