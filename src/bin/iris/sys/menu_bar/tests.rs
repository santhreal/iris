// WHY: a key equivalent shows in the menu bar only on the item that runs
// its action, and a keystroke that does not parse panics at startup.
// Closed here: a binding whose action no menu item runs (a shortcut with
// nothing in the menu bar to show it), two bindings on one keystroke, and
// a keystroke `KeyBinding` cannot parse. Not covered: what AppKit draws,
// which only runs on macOS.

use std::prelude::v1::test;

use gpui::{Action, MenuItem};

use super::{bindings, menus};

/// Every action an item of `items` or of their submenus runs.
fn actions(items: &[MenuItem]) -> Vec<&dyn Action> {
    items
        .iter()
        .flat_map(|item| match item {
            MenuItem::Action { action, .. } => vec![action.as_ref()],
            MenuItem::Submenu(menu) => actions(&menu.items),
            MenuItem::Separator | MenuItem::SystemMenu(_) => Vec::new(),
        })
        .collect()
}

#[test]
fn every_key_equivalent_runs_a_menu_item_on_its_own_keystroke() {
    let menus = menus();
    let shown: Vec<&dyn Action> = menus.iter().flat_map(|m| actions(&m.items)).collect();
    let bindings = bindings();
    for binding in &bindings {
        let action = binding.action();
        assert!(
            shown.iter().any(|a| a.partial_eq(action)),
            "{} runs {}, which no menu item runs",
            binding
                .keystrokes()
                .iter()
                .map(|k| k.unparse())
                .collect::<Vec<_>>()
                .join(" "),
            action.name()
        );
    }
    let mut keys: Vec<String> = bindings
        .iter()
        .map(|b| {
            b.keystrokes()
                .iter()
                .map(|k| k.unparse())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let all = keys.len();
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), all, "{keys:?}");
}
