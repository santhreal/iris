//! The menu bar iris shows on macOS while an app window is open: the
//! iris menu, File, and Window, with the standard key equivalents.
//! `install` sets it on macOS only; other platforms have no app menu
//! bar, and the module compiles everywhere so every build checks it.

use gpui::{App, KeyBinding, Menu, MenuItem, SystemMenuType};

use crate::daemon::{self, Command};

gpui::actions!(
    iris,
    [
        OpenSettings,
        NewScreenshot,
        OpenLibrary,
        HideIris,
        HideOthers,
        ShowAll,
        QuitIris,
        Minimize,
        Zoom,
    ]
);

/// The key equivalents: the ones AppKit apps use for these items. A
/// binding runs before a window's own key handling, which does not see
/// these keys on macOS.
fn bindings() -> [KeyBinding; 5] {
    [
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-h", HideIris, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-q", QuitIris, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]
}

/// Set the menu bar, its key equivalents, and their handlers. Call once
/// at startup.
pub fn install(cx: &mut App) {
    if !cfg!(target_os = "macos") {
        return;
    }
    cx.bind_keys(bindings());
    cx.on_action(|_: &OpenSettings, cx| daemon::run(cx, &Command::Settings));
    cx.on_action(|_: &NewScreenshot, cx| daemon::run(cx, &Command::Capture));
    cx.on_action(|_: &OpenLibrary, cx| daemon::run(cx, &Command::Library));
    cx.on_action(|_: &HideIris, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &QuitIris, cx| daemon::run(cx, &Command::Quit));
    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| window.minimize_window())
                .ok();
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            window.update(cx, |_, window, _| window.zoom_window()).ok();
        }
    });
    cx.set_menus(menus());
}

/// The menus, left to right. AppKit titles the first with the app's
/// name and fills in the windows of the menu named "Window".
fn menus() -> Vec<Menu> {
    vec![
        Menu::new("iris").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide iris", HideIris),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit iris", QuitIris),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Screenshot", NewScreenshot),
            MenuItem::action("Library", OpenLibrary),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Zoom", Zoom),
        ]),
    ]
}

#[cfg(test)]
mod tests;
