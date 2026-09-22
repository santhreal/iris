use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct Config {
    pub screenshots_dir: PathBuf,
    pub recordings_dir: PathBuf,
    pub screenshot_template: String,
    pub record_mic_default: bool,
    pub recording_fps: u32,
    /// Container+codec for recordings: mp4 (H.264), gif, or webm (VP9).
    pub recording_format: RecordingFormat,
    /// H.264 encoder for mp4 recordings: auto probes ffmpeg for
    /// h264_nvenc (GPU offload) and falls back to libx264.
    pub recording_encoder: RecordingEncoder,
    pub show_toast_after_capture: bool,
    pub capture_hotkey: String,
    pub record_hotkey: String,
    pub flash_on_capture: bool,
    pub sound_on_capture: bool,
    /// What a plain click on the toast does.
    pub toast_click_action: ToastClickAction,
    /// Whether dragging the toast starts a file drag-out.
    pub toast_drag_enabled: bool,
    /// Whether the toast pin button is enabled.
    pub toast_pin_enabled: bool,
    /// Whether the toast shows a row of action buttons under the thumb.
    pub toast_show_actions: bool,
    /// How long the toast sits before it dismisses itself.
    pub toast_duration_ms: u32,
    /// Which screen corner the toast lands in.
    pub toast_position: ToastPosition,
    /// Whether a finished capture is placed on the clipboard.
    pub copy_to_clipboard: bool,
    /// Key that cancels the overlay / editor / recording.
    pub cancel_keybind: String,
    /// Key that confirms a pending selection in the overlay.
    pub confirm_keybind: String,
}

/// The action a plain toast click runs.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ToastClickAction {
    /// Open the markup editor over the capture.
    #[default]
    Markup,
    /// Copy the image to the clipboard.
    Copy,
    /// Reveal the file in its folder.
    OpenFolder,
    /// Do nothing; the toast is display-only.
    None,
}

/// The corner the toast anchors to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ToastPosition {
    #[default]
    BottomRight,
    BottomLeft,
    TopRight,
    TopLeft,
}

/// Container and codec for screen recordings.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RecordingFormat {
    /// H.264 in mp4: audio-capable, plays everywhere.
    #[default]
    Mp4,
    /// Animated GIF: no audio, large files, paste-able anywhere.
    Gif,
    /// VP9 in webm: smaller than mp4 at equal quality, no mic track.
    Webm,
}

/// The H.264 encoder used for mp4 recordings.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RecordingEncoder {
    /// Probe ffmpeg for h264_nvenc once and use it when present.
    #[default]
    Auto,
    /// Software x264: works on every host.
    Libx264,
    /// NVIDIA hardware encoder: frees the CPU during capture.
    Nvenc,
}

impl Default for Config {
    fn default() -> Self {
        let user_dirs = directories::UserDirs::new();
        let home = || {
            directories::BaseDirs::new()
                .map(|b| b.home_dir().to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."))
        };
        let pictures = user_dirs
            .as_ref()
            .and_then(|u| u.picture_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| home().join("Pictures"));
        let videos = user_dirs
            .as_ref()
            .and_then(|u| u.video_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| home().join("Videos"));
        Self {
            screenshots_dir: pictures.join("iris"),
            recordings_dir: videos.join("iris"),
            screenshot_template: "{date}_{time}".to_string(),
            record_mic_default: false,
            recording_fps: 30,
            recording_format: RecordingFormat::Mp4,
            recording_encoder: RecordingEncoder::Auto,
            show_toast_after_capture: true,
            capture_hotkey: "Print".to_string(),
            record_hotkey: "Ctrl+Shift+R".to_string(),
            flash_on_capture: true,
            sound_on_capture: true,
            toast_click_action: ToastClickAction::Markup,
            toast_drag_enabled: true,
            toast_pin_enabled: true,
            toast_show_actions: true,
            toast_duration_ms: 5000,
            toast_position: ToastPosition::BottomRight,
            copy_to_clipboard: true,
            cancel_keybind: "Escape".to_string(),
            confirm_keybind: "Enter".to_string(),
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        crate::dirs::config_file()
    }

    /// The shared parsed-config cache, keyed on the file's mtime+len.
    /// `load` reads through it; `store` writes through it.
    fn cache() -> &'static parking_lot::Mutex<(Option<std::time::SystemTime>, u64, Config)> {
        use std::sync::LazyLock;
        static CACHE: LazyLock<parking_lot::Mutex<(Option<std::time::SystemTime>, u64, Config)>> =
            LazyLock::new(|| parking_lot::Mutex::new((None, 0, Config::default())));
        &CACHE
    }

    /// Load the config, re-reading the file only when it changed. The
    /// parsed result is cached behind a mutex keyed on the file's
    /// mtime+len, so the many per-action and per-frame callers share one
    /// stat instead of one parse each.
    pub fn load() -> Self {
        let cache = Self::cache();
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let stamp = std::fs::metadata(&path)
            .and_then(|m| m.modified().map(|t| (Some(t), m.len())))
            .unwrap_or((None, 0));
        {
            let guard = cache.lock();
            if guard.0 == stamp.0 && guard.1 == stamp.1 && stamp.0.is_some() {
                return guard.2.clone();
            }
        }
        let cfg = Self::load_uncached();
        let mut guard = cache.lock();
        *guard = (stamp.0, stamp.1, cfg.clone());
        cfg
    }

    /// Read and parse config.toml unconditionally (migration, defaults,
    /// write-on-missing). `load` wraps this with the mtime cache.
    fn load_uncached() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        // One-time migration from the glint name: carry the existing
        // config over, then read only the new location.
        if !path.exists() {
            if let Some(old) = crate::dirs::legacy_glint_config_file() {
                if old.exists() {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::copy(&old, &path);
                }
            }
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(mut cfg) => {
                    cfg.expand_dirs();
                    return cfg;
                }
                Err(e) => crate::ilog!(
                    "iris: invalid config {}: {e}; using defaults",
                    path.display()
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => crate::ilog!("iris: cannot read {}: {e}; using defaults", path.display()),
        }
        let cfg = Self::default();
        if let Some(parent) = path.parent() {
            if std::fs::create_dir_all(parent).is_ok() {
                if let Ok(text) = toml::to_string_pretty(&cfg) {
                    let _ = std::fs::write(&path, text);
                }
            }
        }
        cfg
    }

    /// Expand a leading `~` in the user-configured directories; without
    /// this a literal "~/iris" in config.toml writes into a directory
    /// named "~" under the process cwd.
    fn expand_dirs(&mut self) {
        let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().to_path_buf()) else {
            return;
        };
        for dir in [&mut self.screenshots_dir, &mut self.recordings_dir] {
            if let Ok(rest) = dir.strip_prefix("~") {
                *dir = home.join(rest);
            }
        }
    }

    /// Persist to config.toml, creating the config dir when missing.
    pub fn store(&self) -> Result<(), String> {
        let path = Self::path().ok_or_else(|| "no config directory".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
        // Keep the load() cache coherent: a save must be visible to the
        // next reader even when the mtime granularity misses the write.
        let stamp = std::fs::metadata(&path)
            .and_then(|m| m.modified().map(|t| (Some(t), m.len())))
            .unwrap_or((None, 0));
        *Self::cache().lock() = (stamp.0, stamp.1, self.clone());
        Ok(())
    }
}

/// Does a stored keybind string ("Escape", "Ctrl+Shift+R", "Print")
/// match a pressed key? `key` is the GPUI key name (lowercase, e.g.
/// "escape", "enter", "a"); `ctrl`/`shift`/`alt`/`super_` are the
/// modifier states. Modifier-only presses never match.
pub fn keybind_matches(
    binding: &str,
    key: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
    super_: bool,
) -> bool {
    let mut want_ctrl = false;
    let mut want_shift = false;
    let mut want_alt = false;
    let mut want_super = false;
    let mut want_key = String::new();
    for part in binding.split('+') {
        match part.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => want_ctrl = true,
            "shift" => want_shift = true,
            "alt" => want_alt = true,
            "super" | "meta" | "cmd" | "win" => want_super = true,
            other => want_key = other.to_string(),
        }
    }
    if want_key.is_empty() {
        return false;
    }
    // Normalize the pressed key the same way the binding is stored.
    let pressed = match key {
        " " => "space",
        "printscreen" => "print",
        k => k,
    };
    pressed.eq_ignore_ascii_case(&want_key)
        && ctrl == want_ctrl
        && shift == want_shift
        && alt == want_alt
        && super_ == want_super
}

// WHY: the class closed here is "config silently lands somewhere the app
// never reads": a wrong ProjectDirs triple, a dropped ~ expansion, or a
// migration that overwrites the new file all look fine until a user's
// settings vanish. Env-mutating tests run serially; XDG vars point at a
// tempdir per test. Not covered: platform dirs on Windows/macOS.
#[cfg(test)]
mod tests {
    use super::*;

    /// Root every iris location in a fresh tempdir via `IRIS_HOME`, on
    /// every platform; returns it so the test can seed files.
    fn isolated_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var(crate::dirs::HOME_ENV, dir.path());
        dir
    }

    /// XDG layout with `IRIS_HOME` cleared, for the legacy-path
    /// migration that only exists on Linux.
    #[cfg(target_os = "linux")]
    fn xdg_without_iris_home() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::remove_var(crate::dirs::HOME_ENV);
        std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
        dir
    }

    #[test]
    #[serial_test::serial]
    fn toml_round_trip_preserves_every_field() {
        let _d = isolated_home();
        let cfg = Config::default();
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.screenshots_dir, cfg.screenshots_dir);
        assert_eq!(back.recordings_dir, cfg.recordings_dir);
        assert_eq!(back.screenshot_template, cfg.screenshot_template);
        assert_eq!(back.recording_fps, cfg.recording_fps);
        assert_eq!(back.capture_hotkey, cfg.capture_hotkey);
        assert_eq!(back.record_hotkey, cfg.record_hotkey);
        assert_eq!(back.toast_click_action, cfg.toast_click_action);
        assert_eq!(back.toast_position, cfg.toast_position);
        assert_eq!(back.toast_pin_enabled, cfg.toast_pin_enabled);
        assert_eq!(back.toast_duration_ms, cfg.toast_duration_ms);
        assert_eq!(back.cancel_keybind, cfg.cancel_keybind);
        assert_eq!(back.confirm_keybind, cfg.confirm_keybind);
    }

    #[test]
    #[serial_test::serial]
    fn old_config_without_new_fields_loads_defaults() {
        // A config.toml written before the toast/keybind fields existed
        // must still load; serde(default) fills them.
        let d = isolated_home();
        let path = Config::path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "recording_fps = 24\n").unwrap();
        let cfg = Config::load();
        assert_eq!(cfg.recording_fps, 24);
        assert_eq!(cfg.toast_click_action, ToastClickAction::Markup);
        assert_eq!(cfg.toast_position, ToastPosition::BottomRight);
        assert!(cfg.toast_pin_enabled);
        assert_eq!(cfg.toast_duration_ms, 5000);
        assert_eq!(cfg.cancel_keybind, "Escape");
        drop(d);
    }

    #[test]
    #[serial_test::serial]
    fn enum_fields_parse_kebab_case() {
        let cfg: Config =
            toml::from_str("toast_click_action = \"copy\"\ntoast_position = \"top-left\"\n")
                .unwrap();
        assert_eq!(cfg.toast_click_action, ToastClickAction::Copy);
        assert_eq!(cfg.toast_position, ToastPosition::TopLeft);
    }

    #[test]
    #[serial_test::serial]
    fn tilde_dirs_expand_to_home() {
        let _d = isolated_home();
        let home = directories::UserDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf();
        let mut cfg = Config {
            screenshots_dir: PathBuf::from("~/shots"),
            recordings_dir: PathBuf::from("~/recs"),
            ..Config::default()
        };
        cfg.expand_dirs();
        assert_eq!(cfg.screenshots_dir, home.join("shots"));
        assert_eq!(cfg.recordings_dir, home.join("recs"));
    }

    #[test]
    #[serial_test::serial]
    fn load_writes_defaults_when_missing() {
        let _d = isolated_home();
        let cfg = Config::load();
        assert!(Config::path().unwrap().exists());
        assert_eq!(cfg.recording_fps, 30);
    }

    #[test]
    #[serial_test::serial]
    fn load_reads_stored_values() {
        let d = isolated_home();
        let path = Config::path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "recording_fps = 24\ncapture_hotkey = \"F9\"\n").unwrap();
        let cfg = Config::load();
        assert_eq!(cfg.recording_fps, 24);
        assert_eq!(cfg.capture_hotkey, "F9");
        drop(d);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[serial_test::serial]
    fn glint_config_migrates_to_iris_path() {
        let d = xdg_without_iris_home();
        let old = d.path().join("config/glint/config.toml");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "recording_fps = 12\n").unwrap();
        let cfg = Config::load();
        assert_eq!(cfg.recording_fps, 12);
        // The copy landed at the iris path, not just in memory.
        assert!(Config::path().unwrap().exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[serial_test::serial]
    fn existing_iris_config_wins_over_glint() {
        let d = xdg_without_iris_home();
        let old = d.path().join("config/glint/config.toml");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "recording_fps = 12\n").unwrap();
        let path = Config::path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "recording_fps = 60\n").unwrap();
        assert_eq!(Config::load().recording_fps, 60);
    }

    #[test]
    #[serial_test::serial]
    fn invalid_toml_falls_back_to_defaults() {
        let _d = isolated_home();
        let path = Config::path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is not toml = = =\n").unwrap();
        let cfg = Config::load();
        assert_eq!(cfg.recording_fps, 30);
    }
}
