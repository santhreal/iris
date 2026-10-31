//! The application menu entry of an AppImage. A deb or an rpm installs
//! `dev.iris.app.desktop` in /usr/share/applications and an autostart
//! entry in /etc/xdg/autostart. An AppImage has no installer: its
//! daemon writes the package's menu entry, with each `Exec` running the
//! AppImage file, in `$XDG_DATA_HOME/applications`, and copies the
//! AppImage's icon beside the account's icon theme. A first start, one
//! with no menu entry yet, also turns on Start at login, as a package
//! install does.

use std::path::{Path, PathBuf};

use crate::sys::xdg;

/// The package's menu entry: each `Exec` runs `iris` from PATH, and
/// `Icon=iris` names the installed theme icon.
const PACKAGE_ENTRY: &str = include_str!("../../../../../../packaging/linux/dev.iris.app.desktop");

/// The entry's file name, the package's: a desktop shows one entry of a
/// name, and the account's overrides the system's.
const NAME: &str = "dev.iris.app.desktop";

/// The icon in the AppImage, under its `usr` directory
/// (build_appimage.sh, `iris_hicolor_icons`).
const ICON: &str = "share/icons/hicolor/256x256/apps/iris.png";

/// Where an account's menu entries and icons are.
struct Places {
    /// `$XDG_DATA_HOME/applications`.
    applications: PathBuf,
    /// The icon the entry names, in `$XDG_DATA_HOME/icons`.
    icon: PathBuf,
}

impl Places {
    fn of_this_account() -> Result<Self, String> {
        let base = directories::BaseDirs::new()
            .ok_or_else(|| "this account has no home directory".to_string())?;
        let data = base.data_dir();
        Ok(Self {
            applications: data.join("applications"),
            icon: data.join("icons/hicolor/256x256/apps/dev.iris.app.png"),
        })
    }
}

/// Give the AppImage `image`, whose iris binary is `exe`, a menu entry.
/// On its first start turn on Start at login unless this account
/// already chose.
pub fn integrate(image: &Path, exe: &Path) {
    let first = Places::of_this_account().and_then(|places| integrate_in(&places, image, exe));
    match first {
        Ok(true) if !crate::sys::autostart::chosen() => {
            if let Err(e) = crate::sys::autostart::set(true) {
                iris_lib::ilog!("iris: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => iris_lib::ilog!("iris: menu entry: {e}"),
    }
}

/// Write the icon and the menu entry of `image` where either differs.
/// True when `places` held no entry before: a first start.
fn integrate_in(places: &Places, image: &Path, exe: &Path) -> Result<bool, String> {
    let path = places.applications.join(NAME);
    let first = !path.exists();
    let text = entry(image, &places.icon)?;
    let usr = exe
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| format!("{} is not in an AppImage's usr/bin", exe.display()))?;
    let source = usr.join(ICON);
    let icon = std::fs::read(&source).map_err(|e| format!("read {}: {e}", source.display()))?;
    if std::fs::read(&places.icon).ok().as_deref() != Some(icon.as_slice()) {
        xdg::write(&places.icon, &icon)?;
    }
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        xdg::write(&path, text.as_bytes())?;
    }
    Ok(first)
}

/// The package's menu entry with each `Exec` running `image`, a
/// `TryExec` that hides the entry once `image` is gone, and `icon` as
/// its icon.
fn entry(image: &Path, icon: &Path) -> Result<String, String> {
    let exec = xdg::exec_arg(image)?;
    let try_exec = xdg::string_value(image)?;
    let icon = xdg::string_value(icon)?;
    let mut text = String::with_capacity(PACKAGE_ENTRY.len() * 2);
    for line in PACKAGE_ENTRY.lines() {
        // `args` is empty for the bare launch, or starts with a space.
        let args = line
            .strip_prefix("Exec=iris")
            .filter(|a| a.is_empty() || a.starts_with(' '));
        if let Some(args) = args {
            text.push_str("Exec=");
            text.push_str(&exec);
            text.push_str(args);
        } else if line == "Icon=iris" {
            text.push_str("Icon=");
            text.push_str(&icon);
        } else {
            text.push_str(line);
        }
        text.push('\n');
        if line == "Type=Application" {
            text.push_str("TryExec=");
            text.push_str(&try_exec);
            text.push('\n');
        }
    }
    Ok(text)
}

// WHY: the classes closed here are "an AppImage has no menu entry",
// "an entry runs iris from PATH, which an AppImage is not on", and "a
// moved AppImage leaves an entry that starts nothing". Not covered:
// whether a desktop shows the entry, and the Start at login a first
// start turns on (crate::sys::autostart has its own tests).
#[cfg(test)]
mod tests;
