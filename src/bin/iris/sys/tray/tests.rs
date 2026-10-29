//! The tray menu table and its dispatch.
//!
//! Every platform draws `MENU` and reports a click as a row id, so a
//! wrong id mapping sends the wrong command on all three at once. These
//! tests walk the table itself: a new row is covered without an edit
//! here. They do not click a live tray.

use futures::channel::mpsc;

use super::{
    command, drawn, pick, rows, set_offer, set_recording, Drawn, Row, When, MENU, TX, UPDATE_LABEL,
};

fn debug(cmd: Option<crate::daemon::Command>) -> String {
    format!("{cmd:?}")
}

/// The record row's label: Linux records a picked window, Windows and
/// macOS the desktop.
const RECORD: &str = if cfg!(target_os = "linux") {
    "Record Window"
} else {
    "Record Screen"
};

#[test]
fn every_row_id_maps_to_that_rows_command() {
    for (id, row) in rows() {
        let want = match row {
            Row::Item { command, .. } => Some(command.clone()),
            Row::Update => Some(crate::daemon::Command::InstallUpdate),
            Row::Separator => None,
        };
        assert_eq!(debug(command(id)), debug(want), "row id {id}");
    }
}

#[test]
fn ids_start_at_one_because_zero_means_no_item() {
    assert_eq!(rows().next().map(|(id, _)| id), Some(1));
    assert!(command(0).is_none());
    assert!(command(MENU.len() + 1).is_none());
    assert!(command(usize::MAX).is_none());
}

#[test]
fn a_pick_sends_the_command_and_a_separator_sends_nothing() {
    let (tx, mut rx) = mpsc::unbounded();
    assert!(TX.set(tx).is_ok(), "only this test sets the channel");
    for (id, row) in rows() {
        pick(id);
        let sent = rx.try_recv().ok();
        match row {
            Row::Item { command, .. } => {
                assert_eq!(debug(sent), debug(Some(command.clone())), "row id {id}")
            }
            Row::Update => assert_eq!(
                debug(sent),
                debug(Some(crate::daemon::Command::InstallUpdate)),
                "install row id {id}"
            ),
            Row::Separator => assert!(sent.is_none(), "separator id {id} sent {sent:?}"),
        }
    }
    pick(0);
    pick(MENU.len() + 1);
    assert!(rx.try_recv().is_err());
}

/// Each label runs the command its usage-chapter row describes, the
/// record row gives way to the stop row while a recording runs, the
/// install row sits with Settings, and Quit sits alone below the
/// separator.
#[test]
fn each_label_runs_its_documented_command() {
    let menu: Vec<String> = MENU
        .iter()
        .map(|row| match row {
            Row::Item {
                label,
                command,
                when,
            } => format!("{label}: {command:?} ({when:?})"),
            Row::Update => format!("{UPDATE_LABEL} <version>: InstallUpdate"),
            Row::Separator => "---".into(),
        })
        .collect();
    assert_eq!(
        menu,
        [
            "New Screenshot: Capture (Always)",
            format!("{RECORD}: RecordToggle (Idle)").as_str(),
            "Stop Recording: RecordStop (Recording)",
            "Library: Library (Always)",
            "Settings: Settings (Always)",
            "Install iris <version>: InstallUpdate",
            "---",
            "Quit: Quit (Always)",
        ]
    );
    assert!(
        !MENU.iter().any(|row| matches!(
            row,
            Row::Item {
                when: When::Recording,
                command: crate::daemon::Command::RecordToggle,
                ..
            }
        )),
        "a row drawn while recording never starts one"
    );
}

/// What a platform draws: the record row while no recording runs and the
/// stop row while one does, every row but the install row while no
/// update is offered, the install row with the offered version while one
/// is, and each drawn row under its `MENU` id in every state, so a click
/// on a menu drawn before a state changed still sends that row's
/// command. The one test that sets the offer and the recording state:
/// the others read `MENU`, not `drawn`.
#[test]
fn the_drawn_rows_follow_the_offer_and_the_recording() {
    let labels = |rows: Vec<(usize, Drawn)>| -> Vec<(usize, String)> {
        rows.into_iter()
            .map(|(id, row)| match row {
                Drawn::Item(label) => (id, label.into_owned()),
                Drawn::Separator => (id, "---".into()),
            })
            .collect()
    };
    let plain: Vec<(usize, String)> = [
        (1, "New Screenshot"),
        (2, RECORD),
        (4, "Library"),
        (5, "Settings"),
        (7, "---"),
        (8, "Quit"),
    ]
    .into_iter()
    .map(|(id, label)| (id, label.to_string()))
    .collect();
    let recording = |rows: &[(usize, String)]| -> Vec<(usize, String)> {
        let mut rows = rows.to_vec();
        rows[1] = (3, "Stop Recording".to_string());
        rows
    };

    set_offer(None);
    set_recording(false);
    assert_eq!(labels(drawn()), plain, "no offer, no recording");

    set_recording(true);
    assert_eq!(labels(drawn()), recording(&plain), "a recording");
    assert_eq!(
        debug(command(3)),
        debug(Some(crate::daemon::Command::RecordStop))
    );
    set_recording(true);
    assert_eq!(labels(drawn()), recording(&plain), "a second start");

    let version = semver::Version::parse("2.1.0-beta.3").unwrap();
    set_offer(Some(&version));
    let mut offered = plain.clone();
    offered.insert(4, (6, "Install iris 2.1.0-beta.3".to_string()));
    assert_eq!(
        labels(drawn()),
        recording(&offered),
        "an offer during a recording"
    );

    set_recording(false);
    assert_eq!(labels(drawn()), offered, "an offer of {version}");
    assert_eq!(
        debug(command(6)),
        debug(Some(crate::daemon::Command::InstallUpdate))
    );

    let newer = semver::Version::parse("2.1.0").unwrap();
    set_offer(Some(&newer));
    offered[4].1 = "Install iris 2.1.0".to_string();
    assert_eq!(labels(drawn()), offered, "a newer offer replaces the label");

    set_offer(None);
    assert_eq!(labels(drawn()), plain, "an offer withdrawn");
}

/// The usage chapter's tray table lists the item labels in menu order. A
/// row's first cell names the item in bold, once per platform label.
#[test]
fn the_usage_chapter_lists_every_item_in_order() {
    let doc = include_str!("../../../../../docs/usage.md");
    let table = doc
        .split_once("The tray menu holds the same actions:")
        .expect("usage.md introduces the tray table")
        .1;
    let documented: Vec<Vec<&str>> = table
        .lines()
        .skip_while(|l| !l.starts_with('|'))
        .take_while(|l| l.starts_with('|'))
        .filter_map(|l| {
            let cell = l.split('|').nth(1)?;
            let labels: Vec<&str> = cell.split("**").skip(1).step_by(2).collect();
            (!labels.is_empty()).then_some(labels)
        })
        .collect();
    let shipped: Vec<&str> = MENU
        .iter()
        .filter_map(|row| match row {
            Row::Item { label, .. } => Some(*label),
            Row::Update => Some(UPDATE_LABEL),
            Row::Separator => None,
        })
        .collect();
    assert_eq!(documented.len(), shipped.len(), "{documented:?}");
    for (labels, label) in documented.iter().zip(&shipped) {
        assert!(labels.contains(label), "{label:?} is not in {labels:?}");
    }
}
