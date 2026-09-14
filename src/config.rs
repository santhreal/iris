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
    pub show_toast_after_capture: bool,
    pub capture_hotkey: String,
    pub record_hotkey: String,
    pub flash_on_capture: bool,
    pub sound_on_capture: bool,
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
            show_toast_after_capture: true,
            capture_hotkey: "Print".to_string(),
            record_hotkey: "Ctrl+Shift+R".to_string(),
            flash_on_capture: true,
            sound_on_capture: true,
        }
    }
}

impl Config {
    fn path() -> Option<PathBuf> {
        directories::ProjectDirs::from("dev", "iris", "iris")
            .map(|p| p.config_dir().join("config.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        // One-time migration from the glint name: carry the existing
        // config over, then read only the new location.
        if !path.exists() {
            if let Some(old) = directories::ProjectDirs::from("dev", "glint", "glint")
                .map(|p| p.config_dir().join("config.toml"))
            {
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
                Err(e) => eprintln!("iris: invalid config {}: {e}; using defaults", path.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("iris: cannot read {}: {e}; using defaults", path.display()),
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
        let text = toml::to_string_pretty(self)
            .map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
    }
}
