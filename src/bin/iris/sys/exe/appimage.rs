//! The AppImage this iris runs from. The AppImage runtime mounts the
//! AppImage at `$APPDIR` and sets `$APPIMAGE` to its file; a program it
//! starts passes both on to its children. An iris started by another
//! AppImage's program, a terminal or an editor, has that AppImage's two
//! and runs from outside its mount: that AppImage is not iris, and
//! neither an update nor an autostart entry may name it.

use std::path::{Path, PathBuf};

/// The AppImage `exe` runs from, by the `$APPIMAGE` and `$APPDIR` this
/// process inherited; None when `exe` is outside the mount `$APPDIR`
/// names.
pub fn appimage(exe: &Path) -> Option<PathBuf> {
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    appimage_of(exe, env("APPIMAGE"), env("APPDIR"))
}

/// `appimage` when `exe` is inside the mount `appdir`.
fn appimage_of(exe: &Path, appimage: Option<PathBuf>, appdir: Option<PathBuf>) -> Option<PathBuf> {
    let appdir = appdir?;
    let appdir = std::fs::canonicalize(&appdir).unwrap_or(appdir);
    // An empty or a root `appdir` holds every path.
    let mount = appdir.is_absolute() && appdir.parent().is_some();
    (mount && exe.starts_with(appdir))
        .then_some(appimage)
        .flatten()
}

// WHY: the class closed here is "an iris started from another
// AppImage's program takes that AppImage for its own": the updater
// overwrote it with iris, and Start at login wrote an entry that
// started it. Both read the AppImage here. Not covered: the
// environment `appimage` reads.
#[cfg(test)]
mod tests;
