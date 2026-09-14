use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 200;
const THUMB_WIDTH: u32 = 320;

/// One capture in the library: the image plus its cached thumbnail.
#[derive(Serialize, Deserialize, Clone)]
pub struct CaptureEntry {
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub width: u32,
    pub height: u32,
    pub created_ms: i64,
}

/// The shared data directory (dev.iris.app; the GPUI side passes the
/// same path via directories::ProjectDirs with an empty organization).
pub fn app_data_dir() -> Result<PathBuf, String> {
    let dir = directories::ProjectDirs::from("", "", "dev.iris.app")
        .ok_or("no project data dir")?
        .data_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
    Ok(dir)
}

/// The shared cache directory (thumbnails, frozen frames).
pub fn app_cache_dir() -> Result<PathBuf, String> {
    let dir = directories::ProjectDirs::from("", "", "dev.iris.app")
        .ok_or("no project cache dir")?
        .cache_dir()
        .to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app cache dir: {e}"))?;
    Ok(dir)
}

fn store_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join("library.json"))
}

fn thumbs_dir() -> Result<PathBuf, String> {
    let dir = app_cache_dir()?.join("thumbs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create thumbs dir: {e}"))?;
    Ok(dir)
}

/// FNV-1a over the path bytes: stable across restarts, no dependency.
fn path_key(path: &Path) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn read_store() -> Vec<CaptureEntry> {
    let Ok(path) = store_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("iris: corrupt library.json, starting fresh: {e}");
        Vec::new()
    })
}

fn write_store(entries: &[CaptureEntry]) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write library.json: {e}"))?;
    Ok(())
}

/// Register a fresh capture: build its thumbnail, prepend it, cap the list.
pub fn add(path: &Path, width: u32, height: u32) -> Result<CaptureEntry, String> {
    let img = image::open(path).map_err(|e| format!("open capture for thumbnail: {e}"))?;
    let scale = THUMB_WIDTH as f64 / width as f64;
    let thumb_img = img.resize(
        THUMB_WIDTH,
        (height as f64 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    );
    let thumb = thumbs_dir()?.join(format!("{}.png", path_key(path)));
    thumb_img
        .save(&thumb)
        .map_err(|e| format!("save thumbnail: {e}"))?;

    let entry = CaptureEntry {
        path: path.to_path_buf(),
        thumb,
        width,
        height,
        created_ms: chrono::Local::now().timestamp_millis(),
    };
    let mut entries = read_store();
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry.clone());
    entries.truncate(MAX_ENTRIES);
    write_store(&entries)?;
    Ok(entry)
}

pub fn list() -> Vec<CaptureEntry> {
    let entries = read_store();
    // Prune entries whose file vanished (user moved/deleted it).
    let (alive, dead): (Vec<_>, Vec<_>) = entries.into_iter().partition(|e| e.path.exists());
    if !dead.is_empty() {
        let _ = write_store(&alive);
    }
    alive
}

/// Remove a capture everywhere: entry, thumbnail, and the image file.
pub fn delete(path: &Path) -> Result<(), String> {
    let mut entries = read_store();
    let before = entries.len();
    entries.retain(|e| e.path != path);
    if entries.len() == before {
        return Err(format!("{} is not in the library", path.display()));
    }
    write_store(&entries)?;
    let _ = std::fs::remove_file(thumbs_dir()?.join(format!("{}.png", path_key(path))));
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| format!("delete {}: {e}", path.display()))?;
    }
    Ok(())
}

// WHY: the class closed here is "the library loses or corrupts captures":
// a store that does not round-trip, a delete that leaves the file, a cap
// that keeps the wrong end, or a path_key that collides all surface as
// missing thumbnails or vanished shots. Env-mutating tests run serially
// with XDG pointed at a tempdir. Not covered: thumbnail pixel content.
#[cfg(test)]
mod tests {
    use super::*;

    /// Point XDG data/cache at a fresh tempdir; returns it for file seeds.
    fn xdg() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
        std::env::set_var("XDG_CACHE_HOME", dir.path().join("cache"));
        dir
    }

    /// A real PNG on disk inside `dir`; returns its path.
    fn png(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([1, 2, 3, 255]));
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn path_key_is_stable_and_path_sensitive() {
        let a = Path::new("/tmp/one.png");
        let b = Path::new("/tmp/two.png");
        assert_eq!(path_key(a), path_key(a));
        assert_ne!(path_key(a), path_key(b));
        assert_eq!(path_key(a).len(), 16);
    }

    #[test]
    #[serial_test::serial]
    fn add_list_delete_round_trips() {
        let d = xdg();
        let shot = png(d.path(), "a.png");
        let entry = add(&shot, 8, 6).unwrap();
        assert!(entry.thumb.exists());
        assert_eq!(list().len(), 1);
        delete(&shot).unwrap();
        assert!(list().is_empty());
        assert!(!entry.thumb.exists());
        assert!(!shot.exists());
    }

    #[test]
    #[serial_test::serial]
    fn delete_unknown_path_errors() {
        let _d = xdg();
        assert!(delete(Path::new("/nonexistent.png")).is_err());
    }

    #[test]
    #[serial_test::serial]
    fn list_prunes_entries_whose_file_vanished() {
        let d = xdg();
        let shot = png(d.path(), "gone.png");
        add(&shot, 8, 6).unwrap();
        std::fs::remove_file(&shot).unwrap();
        assert!(list().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn readding_same_path_replaces_not_duplicates() {
        let d = xdg();
        let shot = png(d.path(), "dup.png");
        add(&shot, 8, 6).unwrap();
        add(&shot, 8, 6).unwrap();
        assert_eq!(list().len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn corrupt_store_starts_fresh() {
        let d = xdg();
        let store = app_data_dir().unwrap().join("library.json");
        std::fs::write(&store, "{not json").unwrap();
        assert!(list().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn store_caps_at_max_entries() {
        let d = xdg();
        for i in 0..5 {
            let shot = png(d.path(), &format!("s{i}.png"));
            add(&shot, 8, 6).unwrap();
        }
        // Push past the cap directly: 200 adds of real PNGs is slow.
        let mut entries = read_store();
        for i in 0..MAX_ENTRIES + 10 {
            entries.push(CaptureEntry {
                path: d.path().join(format!("extra{i}.png")),
                thumb: PathBuf::new(),
                width: 1,
                height: 1,
                created_ms: i as i64,
            });
        }
        entries.truncate(MAX_ENTRIES);
        write_store(&entries).unwrap();
        assert_eq!(read_store().len(), MAX_ENTRIES);
    }
}
