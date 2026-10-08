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
        // 22x22: white shutter glyph on transparent. Plain ARGB32.
        // Static: ksni re-queries the pixmap on every tray update, so
        // the raster is built once and cloned.
        static ICON: std::sync::LazyLock<ksni::Icon> = std::sync::LazyLock::new(|| {
            let n = 22;
            let mut data = vec![0u8; n * n * 4];
            for y in 4..18 {
                for x in 4..18 {
                    let dx = x as i32 - 11;
                    let dy = y as i32 - 11;
                    let ring = (36..=49).contains(&(dx * dx + dy * dy));
                    let dot = dx * dx + dy * dy <= 4;
                    if ring || dot {
                        let i = (y * n + x) * 4;
                        data[i..i + 4].copy_from_slice(&[242, 242, 244, 255]);
                    }
                }
            }
            ksni::Icon {
                width: n as i32,
                height: n as i32,
                data,
            }
        });
        vec![ICON.clone()]
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
