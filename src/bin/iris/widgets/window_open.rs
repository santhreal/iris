//! Opening a window. Every iris window opens through `open_window`,
//! which sets the app id of the desktop entry and the window's title.

use gpui::{App, Entity, Render, Window, WindowHandle, WindowOptions};

/// Open a window titled `title` with the app id `iris_lib::APP_ID`: the
/// X11 class and Wayland app id that docks, task switchers, and window
/// rules match to the desktop entry `dev.iris.app.desktop`. Windows
/// differ by title, which taskbars and window lists show. `options`
/// sets everything else. clippy.toml disallows `App::open_window` and
/// `AsyncApp::open_window` outside this function.
#[allow(clippy::disallowed_methods)] // the one caller of App::open_window
pub fn open_window<V: 'static + Render>(
    cx: &mut App,
    title: &str,
    options: WindowOptions,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> gpui::Result<WindowHandle<V>> {
    let options = WindowOptions {
        app_id: Some(iris_lib::APP_ID.to_owned()),
        ..options
    };
    cx.open_window(options, |window, cx| {
        window.set_window_title(title);
        build(window, cx)
    })
}
