use std::sync::OnceLock;

use super::Drawn;

/// The thread that holds the tray's handle: `redraw` wakes it.
static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();

pub(super) struct IrisTray;

impl ksni::Tray for IrisTray {
    // A primary click opens the menu, as on Windows and macOS. The item
    // reports ItemIsMenu and answers Activate with the error that
    // GNOME's AppIndicator extension and Plasma read as "show the menu".
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "iris".into()
    }
    fn title(&self) -> String {
        "iris".into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // ksni re-queries the pixmaps on every tray update, so they are
        // decoded once and cloned.
        static ICONS: std::sync::LazyLock<Vec<ksni::Icon>> = std::sync::LazyLock::new(pixmaps);
        ICONS.clone()
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        super::drawn()
            .into_iter()
            .map(|(id, row)| match row {
                Drawn::Item(label) => StandardItem {
                    label: label.into_owned(),
                    activate: Box::new(move |_| super::pick(id)),
                    ..Default::default()
                }
                .into(),
                Drawn::Separator => MenuItem::Separator,
            })
            .collect()
    }

    fn watcher_online(&self) {
        iris_lib::ilog!("iris: tray registered");
    }

    // Keep the service up: the tray registers the moment a watcher
    // (re)appears, e.g. a panel that starts after a login autostart.
    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        iris_lib::ilog!("iris: tray waiting for a StatusNotifierWatcher: {reason:?}");
        true
    }
}

/// The tray glyph, white on transparency, at each size
/// `packaging/gen_icons.py` renders: the panel picks the one that fits.
const GLYPHS: [&[u8]; 5] = [
    include_bytes!("../../../../../packaging/icons/tray-16.png"),
    include_bytes!("../../../../../packaging/icons/tray-22.png"),
    include_bytes!("../../../../../packaging/icons/tray-24.png"),
    include_bytes!("../../../../../packaging/icons/tray-32.png"),
    include_bytes!("../../../../../packaging/icons/tray-48.png"),
];

/// `png` as a StatusNotifierItem pixmap: ARGB32, each pixel's bytes in
/// network order. None for a PNG that does not decode.
fn pixmap(png: &[u8]) -> Option<ksni::Icon> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| iris_lib::ilog!("iris: tray: glyph does not decode: {e}"))
        .ok()?
        .into_rgba8();
    let (width, height) = img.dimensions();
    let data = img
        .pixels()
        .flat_map(|p| {
            let [r, g, b, a] = p.0;
            [a, r, g, b]
        })
        .collect();
    Some(ksni::Icon {
        width: i32::try_from(width).ok()?,
        height: i32::try_from(height).ok()?,
        data,
    })
}

/// Every glyph that decodes, as pixmaps.
pub(super) fn pixmaps() -> Vec<ksni::Icon> {
    GLYPHS.iter().filter_map(|png| pixmap(png)).collect()
}

/// Spawn the StatusNotifierItem on its own thread and keep it
/// registered for the process lifetime. A watcher that is absent at
/// startup does not fail the spawn: the item registers when one
/// appears. Only a session bus failure degrades to a log line. The
/// thread sleeps until `redraw` wakes it, and each wake sends the menu
/// again.
pub(super) fn spawn() {
    std::thread::spawn(|| {
        use ksni::blocking::TrayMethods;
        match IrisTray.assume_sni_available(true).spawn() {
            Err(e) => iris_lib::ilog!("iris: tray unavailable: {e}"),
            Ok(handle) => {
                let _ = THREAD.set(std::thread::current());
                loop {
                    std::thread::park();
                    // An update with no change still makes ksni read
                    // `menu` again and signal the new layout.
                    handle.update(|_| {});
                }
            }
        }
    });
}

/// Send the menu to the panel again. Before the tray thread holds its
/// handle this does nothing: the menu the panel reads first is drawn
/// from the current rows.
pub(super) fn redraw() {
    if let Some(thread) = THREAD.get() {
        thread.unpark();
    }
}
