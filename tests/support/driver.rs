//! A Vulkan driver a case adds to a daemon's Vulkan loader.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// tests/fixtures/display_driver.rs built into a shared library, the
/// manifest the Vulkan loader finds it through, and the file it records
/// each load and each device enumeration in.
pub struct Driver {
    manifest: PathBuf,
    pub log: PathBuf,
}

impl Driver {
    pub fn build(dir: &Path) -> Driver {
        let library = dir.join("libdisplay_driver.so");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/display_driver.rs");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let built = Command::new(&rustc)
            .args(["--edition", "2021", "--crate-type", "cdylib"])
            .args([
                "--crate-name",
                "display_driver",
                "-C",
                "strip=symbols",
                "-o",
            ])
            .arg(&library)
            .arg(&source)
            .status()
            .unwrap_or_else(|e| panic!("run {rustc:?}: {e}"));
        assert!(
            built.success(),
            "{rustc:?} could not build {}",
            source.display()
        );
        // A driver without vkEnumerateInstanceVersion is a Vulkan 1.0
        // driver; the loader warns of one whose manifest states more.
        let manifest = dir.join("display_driver.json");
        let json = serde_json::json!({
            "file_format_version": "1.0.0",
            "ICD": { "library_path": library, "api_version": "1.0.0" },
        });
        std::fs::write(&manifest, json.to_string()).unwrap();
        Driver {
            manifest,
            log: dir.join("display_driver.log"),
        }
    }

    /// Add the driver to the ones the daemon's Vulkan loader loads.
    pub fn add_to(&self, cmd: &mut Command) {
        // The loader ignores VK_ADD_DRIVER_FILES once VK_ICD_FILENAMES is
        // set, as the harness sets it where lavapipe is installed.
        let listed = cmd
            .get_envs()
            .find(|(key, _)| *key == "VK_ICD_FILENAMES")
            .and_then(|(_, value)| value.map(OsString::from));
        match listed {
            Some(mut list) => {
                list.push(":");
                list.push(&self.manifest);
                cmd.env("VK_ICD_FILENAMES", list);
            }
            None => {
                cmd.env("VK_ADD_DRIVER_FILES", &self.manifest);
            }
        }
        cmd.env("IRIS_TEST_DRIVER_LOG", &self.log);
    }
}
