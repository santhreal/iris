//! The tray menu table and its dispatch.
//!
//! Every platform draws `MENU` and reports a click as a row id, so a
//! wrong id mapping sends the wrong command on all three at once. These
//! tests walk the table itself: a new row is covered without an edit
//! here. They do not click a live tray.

use futures::channel::mpsc;

use super::{command, drawn, pick, rows, set_offer, Drawn, Row, MENU, TX, UPDATE_LABEL};

fn debug(cmd: Option<crate::daemon::Command>) -> String {
    format!("{cmd:?}")
}

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
/// install row sits with Settings, and Quit sits alone below the
/// separator.
#[test]
fn each_label_runs_its_documented_command() {
    let menu: Vec<String> = MENU
        .iter()
        .map(|row| match row {
            Row::Item { label, command } => format!("{label}: {command:?}"),
            Row::Update => format!("{UPDATE_LABEL} <version>: InstallUpdate"),
            Row::Separator => "---".into(),
        })
        .collect();
    assert_eq!(
        menu,
        [
            "Capture: Capture",
            "Record window: RecordToggle",
            "Library: Library",
            "Settings: Settings",
            "Install iris <version>: InstallUpdate",
            "---",
            "Quit: Quit",
        ]
    );
}

/// What a platform draws: every row but the install row while no update
/// is offered, the install row with the offered version while one is,
/// and each drawn row under its `MENU` id either way, so a click on a
/// menu drawn before an offer changed still sends that row's command.
/// The one test that sets the offer: the others read `MENU`, not
/// `drawn`.
#[test]
fn the_install_row_is_drawn_only_while_an_update_is_offered() {
    let labels = |rows: Vec<(usize, Drawn)>| -> Vec<(usize, String)> {
        rows.into_iter()
            .map(|(id, row)| match row {
                Drawn::Item(label) => (id, label.into_owned()),
                Drawn::Separator => (id, "---".into()),
            })
            .collect()
    };
    let plain: Vec<(usize, String)> = [
        (1, "Capture"),
        (2, "Record window"),
        (3, "Library"),
        (4, "Settings"),
        (6, "---"),
        (7, "Quit"),
    ]
    .into_iter()
    .map(|(id, label)| (id, label.to_string()))
    .collect();

    set_offer(None);
    assert_eq!(labels(drawn()), plain, "no offer");

    let version = semver::Version::parse("2.1.0-beta.3").unwrap();
    set_offer(Some(&version));
    let mut offered = plain.clone();
    offered.insert(4, (5, "Install iris 2.1.0-beta.3".to_string()));
    assert_eq!(labels(drawn()), offered, "an offer of {version}");
    assert_eq!(
        debug(command(5)),
        debug(Some(crate::daemon::Command::InstallUpdate))
    );

    let newer = semver::Version::parse("2.1.0").unwrap();
    set_offer(Some(&newer));
    offered[4].1 = "Install iris 2.1.0".to_string();
    assert_eq!(labels(drawn()), offered, "a newer offer replaces the label");

    set_offer(None);
    assert_eq!(labels(drawn()), plain, "an offer withdrawn");
}

/// The usage chapter's tray table lists the item labels in menu order.
#[test]
fn the_usage_chapter_lists_every_item_in_order() {
    let doc = include_str!("../../../../../docs/usage.md");
    let table = doc
        .split_once("The tray menu holds the same actions:")
        .expect("usage.md introduces the tray table")
        .1;
    let documented: Vec<&str> = table
        .lines()
        .skip_while(|l| !l.starts_with('|'))
        .take_while(|l| l.starts_with('|'))
        .filter_map(|l| {
            l.strip_prefix("| **")?
                .split_once("**")
                .map(|(label, _)| label)
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
    assert_eq!(documented, shipped);
}
