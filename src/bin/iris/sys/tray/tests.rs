//! The tray menu table and its dispatch.
//!
//! Every platform draws `MENU` and reports a click as a row id, so a
//! wrong id mapping sends the wrong command on all three at once. These
//! tests walk the table itself: a new row is covered without an edit
//! here. They do not click a live tray.

use futures::channel::mpsc;

use super::{command, pick, rows, Row, MENU, TX};

fn debug(cmd: Option<crate::daemon::Command>) -> String {
    format!("{cmd:?}")
}

#[test]
fn every_row_id_maps_to_that_rows_command() {
    for (id, row) in rows() {
        let want = match row {
            Row::Item { command, .. } => Some(command.clone()),
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
            Row::Separator => assert!(sent.is_none(), "separator id {id} sent {sent:?}"),
        }
    }
    pick(0);
    pick(MENU.len() + 1);
    assert!(rx.try_recv().is_err());
}

/// Each label runs the command its usage-chapter row describes, and
/// Quit sits alone below the separator.
#[test]
fn each_label_runs_its_documented_command() {
    let menu: Vec<String> = MENU
        .iter()
        .map(|row| match row {
            Row::Item { label, command } => format!("{label}: {command:?}"),
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
            "---",
            "Quit: Quit",
        ]
    );
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
            Row::Separator => None,
        })
        .collect();
    assert_eq!(documented, shipped);
}
