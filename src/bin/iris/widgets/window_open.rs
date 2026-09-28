//! Opening a window. Every iris window opens through `open_window`,
//! which sets the app id of the desktop entry, the window's title, and
//! the frame rate of its animations while it has no focus.

use gpui::{App, Entity, Render, Window, WindowHandle, WindowOptions};

/// Open a window titled `title` with the app id `iris_lib::APP_ID`: the
/// X11 class and Wayland app id that docks, task switchers, and window
/// rules match to the desktop entry `dev.iris.app.desktop`. Windows
/// differ by title, which taskbars and window lists show.
///
/// Animations run at the display rate whether or not the window has
/// focus. GPUI caps an unfocused window's animation frames to 30 per
/// second by default. Pop-ups (the toast, notices, the recording chip,
/// the capture flash) never take focus, so the cap halved the frame rate
/// of every one of their animations; a normal window animates a new
/// capture while another application has focus. Every iris animation
/// settles and requests no frames at rest, so the cap saves nothing.
///
/// `options` sets everything else. clippy.toml disallows
/// `App::open_window` and `AsyncApp::open_window` outside this function.
#[allow(clippy::disallowed_methods)] // the one caller of App::open_window
pub fn open_window<V: 'static + Render>(
    cx: &mut App,
    title: &str,
    options: WindowOptions,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> gpui::Result<WindowHandle<V>> {
    cx.open_window(iris_options(options), |window, cx| {
        window.set_window_title(title);
        build(window, cx)
    })
}

/// `options` with the app id and frame rate every iris window opens with.
fn iris_options(options: WindowOptions) -> WindowOptions {
    WindowOptions {
        app_id: Some(iris_lib::APP_ID.to_owned()),
        inactive_frame_interval: None,
        ..options
    }
}

#[cfg(test)]
mod tests;
