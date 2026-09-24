use futures::channel::mpsc::UnboundedSender;

use crate::daemon::Command;

#[cfg(target_os = "linux")]
pub(super) struct IrisTray {
    pub(super) tx: UnboundedSender<Command>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for IrisTray {
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
        let item = |label: &str, cmd: Command| {
            let tx = self.tx.clone();
            StandardItem {
                label: label.to_string(),
                activate: Box::new(move |_| {
                    let _ = tx.unbounded_send(cmd.clone());
                }),
                ..Default::default()
            }
            .into()
        };
        vec![
            item("Capture", Command::Capture),
            item("Record window", Command::RecordToggle),
            item("Library", Command::Library),
            item("Settings", Command::Settings),
            MenuItem::Separator,
            item("Quit", Command::Quit),
        ]
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
/// appears. Only a session bus failure degrades to a log line.
pub(super) fn spawn(tx: futures::channel::mpsc::UnboundedSender<Command>) {
    std::thread::spawn(move || {
        use ksni::blocking::TrayMethods;
        let tray = IrisTray { tx };
        match tray.assume_sni_available(true).spawn() {
            Err(e) => iris_lib::ilog!("iris: tray unavailable: {e}"),
            Ok(handle) => {
                let _keep = handle;
                loop {
                    std::thread::park();
                }
            }
        }
    });
}
