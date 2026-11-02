use std::path::Path;
use std::sync::OnceLock;

use super::Drawn;

/// The thread that holds the tray's handle: `redraw` wakes it.
static THREAD: OnceLock<std::thread::Thread> = OnceLock::new();

pub(super) struct IrisTray {
    /// The directory that holds `SYMBOLIC_NAME`.svg, for the panel to
    /// look the glyph up in. None when it could not be written: the
    /// panel then draws `icon_pixmap`.
    theme: Option<String>,
}

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
    // A panel that finds the name in the path draws the symbolic glyph
    // in its own foreground color; one that does not draws the pixmaps.
    fn icon_theme_path(&self) -> String {
        self.theme.clone().unwrap_or_default()
    }
    fn icon_name(&self) -> String {
        match self.theme {
            Some(_) => SYMBOLIC_NAME.into(),
            None => String::new(),
        }
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

/// The tray glyph as a symbolic icon. GTK panels and GNOME Shell fill a
/// `-symbolic` icon with the panel's foreground color and Plasma sets
/// its `ColorScheme-Text` color, so the glyph reads on a light panel as
/// on a dark one, where the white pixmaps vanish on a light one.
const SYMBOLIC: &str = include_str!("../../../../../packaging/icons/tray-symbolic.svg");

/// The name a panel looks the symbolic glyph up by: its file name in
/// the item's `IconThemePath`, less `.svg`.
pub(super) const SYMBOLIC_NAME: &str = "iris-tray-symbolic";

/// Write the symbolic glyph into `dir` and return `dir` as the item's
/// `IconThemePath`. A file that already holds the glyph is not written
/// again.
pub(super) fn symbolic_theme(dir: &Path) -> Result<String, String> {
    let theme = dir
        .to_str()
        .ok_or_else(|| format!("{} is not UTF-8", dir.display()))?;
    let path = dir.join(format!("{SYMBOLIC_NAME}.svg"));
    if std::fs::read(&path).is_ok_and(|held| held == SYMBOLIC.as_bytes()) {
        return Ok(theme.to_owned());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    std::fs::write(&path, SYMBOLIC).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(theme.to_owned())
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
        let theme = iris_lib::dirs::cache_dir()
            .ok_or_else(|| "no cache directory for this user".to_string())
            .and_then(|cache| symbolic_theme(&cache.join("tray")))
            .map_err(|e| iris_lib::ilog!("iris: tray: the panel draws the white glyph: {e}"))
            .ok();
        match (IrisTray { theme }).assume_sni_available(true).spawn() {
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
