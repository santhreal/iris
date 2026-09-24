//! Every on-disk location iris reads or writes.
//!
//! Defaults follow each platform's conventions through `directories`:
//! XDG on Linux, `%APPDATA%`/`%LOCALAPPDATA%` on Windows, and
//! `~/Library` on macOS. When `IRIS_HOME` is set, every location is a
//! subdirectory of it instead, on every platform:
//!
//! | location | default (Linux)            | with `IRIS_HOME`           |
//! |----------|----------------------------|----------------------------|
//! | config   | `$XDG_CONFIG_HOME/iris`    | `$IRIS_HOME/config`        |
//! | data     | `$XDG_DATA_HOME/dev.iris.app`  | `$IRIS_HOME/data`      |
//! | cache    | `$XDG_CACHE_HOME/dev.iris.app` | `$IRIS_HOME/cache`     |
//! | log      | `$XDG_STATE_HOME/iris`     | `$IRIS_HOME/state`         |

use std::path::PathBuf;

/// Environment variable that roots every iris location in one directory.
pub const HOME_ENV: &str = "IRIS_HOME";

fn home_override() -> Option<PathBuf> {
    std::env::var_os(HOME_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn app_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("", "", crate::APP_ID)
}

/// `config.toml`.
pub fn config_file() -> Option<PathBuf> {
    let dir = match home_override() {
        Some(home) => home.join("config"),
        None => directories::ProjectDirs::from("dev", "iris", "iris")?
            .config_dir()
            .to_path_buf(),
    };
    Some(dir.join("config.toml"))
}

/// The config file of the app's former name, migrated once by
/// `Config::load`. `None` under `IRIS_HOME`: a rooted install has no
/// legacy location.
pub fn legacy_glint_config_file() -> Option<PathBuf> {
    if home_override().is_some() {
        return None;
    }
    directories::ProjectDirs::from("dev", "glint", "glint")
        .map(|p| p.config_dir().join("config.toml"))
}

/// Library store and other persistent data.
pub fn data_dir() -> Option<PathBuf> {
    match home_override() {
        Some(home) => Some(home.join("data")),
        None => app_dirs().map(|p| p.data_dir().to_path_buf()),
    }
}

/// Regenerable files: thumbnails, frozen frames.
pub fn cache_dir() -> Option<PathBuf> {
    match home_override() {
        Some(home) => Some(home.join("cache")),
        None => app_dirs().map(|p| p.cache_dir().to_path_buf()),
    }
}

/// `iris.log`. Linux uses the XDG state dir; platforms without one use
/// the config dir. The current directory is the last resort, so the
/// log path is always defined.
pub fn log_file() -> PathBuf {
    let dir = match home_override() {
        Some(home) => home.join("state"),
        None => directories::BaseDirs::new()
            .and_then(|b| {
                b.state_dir()
                    .map(|p| p.to_path_buf())
                    .or_else(|| Some(b.config_dir().to_path_buf()))
            })
            .map(|b| b.join("iris"))
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    dir.join("iris.log")
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "a location escapes IRIS_HOME". Every
    // path accessor must resolve under the override, or tests and rooted
    // installs write into the real per-user profile. Not covered: the
    // platform defaults, which belong to the `directories` crate.
    use super::*;

    #[test]
    #[serial_test::serial]
    fn every_location_resolves_under_iris_home() {
        let root = tempfile::tempdir().unwrap();
        std::env::set_var(HOME_ENV, root.path());
        let paths = [
            config_file().unwrap(),
            data_dir().unwrap(),
            cache_dir().unwrap(),
            log_file(),
        ];
        assert_eq!(legacy_glint_config_file(), None);
        std::env::remove_var(HOME_ENV);
        for p in paths {
            assert!(
                p.starts_with(root.path()),
                "{} escapes IRIS_HOME",
                p.display()
            );
        }
    }

    #[test]
    #[serial_test::serial]
    fn empty_iris_home_is_ignored() {
        std::env::set_var(HOME_ENV, "");
        let data = data_dir();
        std::env::remove_var(HOME_ENV);
        assert_eq!(data, app_dirs().map(|p| p.data_dir().to_path_buf()));
    }
}
