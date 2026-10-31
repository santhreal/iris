//! Opening a window. Every iris window opens through `open_window`,
//! which sets the app id of the desktop entry, the window's title, the
//! frame rate of its animations while it has no focus, and whether iris
//! shows in the macOS Dock and menu bar.

use std::cell::{Cell, RefCell};

use gpui::{
    ActivationPolicy, AnyWindowHandle, App, Entity, Render, Window, WindowHandle, WindowId,
    WindowOptions,
};

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
/// A window that can minimize (home, library, settings, the editor) is
/// an app window: while one is open iris has a Dock icon and a menu bar
/// on macOS, and with none open it has neither, as the capture overlay,
/// the toast, notices, pins, and the recording chip are not app
/// windows.
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
    let app_window = options.is_minimizable;
    #[cfg(target_os = "linux")]
    let options = WindowOptions {
        icon: options.icon.or_else(|| app_window.then(app_icon).flatten()),
        ..options
    };
    let handle = cx.open_window(iris_options(options), |window, cx| {
        window.set_window_title(title);
        build(window, cx)
    })?;
    if app_window {
        watch_closes(cx);
        let id = AnyWindowHandle::from(handle).window_id();
        let change = APP_WINDOWS.with(|w| w.borrow_mut().opened(id));
        if let Some(policy) = change {
            cx.set_activation_policy(policy);
        }
    }
    Ok(handle)
}

/// The PNG of the icon an X11 app window sets as `_NET_WM_ICON`.
#[cfg(target_os = "linux")]
const ICON_PNG: &[u8] = include_bytes!("../../../../packaging/icons/iris-128.png");

/// The app icon an X11 taskbar or window switcher shows for an app
/// window when no installed desktop entry matches its class, as for a
/// source build or an AppImage before its first start. Decoded once.
#[cfg(target_os = "linux")]
fn app_icon() -> Option<std::sync::Arc<image::RgbaImage>> {
    static ICON: std::sync::OnceLock<Option<std::sync::Arc<image::RgbaImage>>> =
        std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
            .map(|icon| std::sync::Arc::new(icon.to_rgba8()))
            .map_err(|e| iris_lib::ilog!("iris: decode the window icon: {e}"))
            .ok()
    })
    .clone()
}

thread_local! {
    /// The app windows open now. GPUI windows live on the main thread.
    static APP_WINDOWS: RefCell<AppWindows> = RefCell::default();
    /// Whether `watch_closes` has subscribed.
    static WATCHING: Cell<bool> = const { Cell::new(false) };
}

/// Subscribe once to window closes, which take a closed app window out
/// of `APP_WINDOWS`.
fn watch_closes(cx: &mut App) {
    if WATCHING.with(|w| w.replace(true)) {
        return;
    }
    cx.on_window_closed(|cx, id| {
        if let Some(policy) = APP_WINDOWS.with(|w| w.borrow_mut().closed(id)) {
            cx.set_activation_policy(policy);
        }
    })
    .detach();
}

/// The open app windows, and the activation policy each open and close
/// changes to.
#[derive(Default)]
struct AppWindows(Vec<WindowId>);

impl AppWindows {
    /// Record `id` open. The first app window puts iris in the Dock.
    fn opened(&mut self, id: WindowId) -> Option<ActivationPolicy> {
        let first = self.0.is_empty();
        if !self.0.contains(&id) {
            self.0.push(id);
        }
        first.then_some(ActivationPolicy::Regular)
    }

    /// Record `id` closed. The last app window to close takes iris out
    /// of the Dock; any other window changes nothing.
    fn closed(&mut self, id: WindowId) -> Option<ActivationPolicy> {
        let before = self.0.len();
        self.0.retain(|w| *w != id);
        (before == 1 && self.0.is_empty()).then_some(ActivationPolicy::Accessory)
    }
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
