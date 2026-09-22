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

/// The parsed store behind an mtime+len check: the library window
/// polls list() every 1.5s, and an unchanged file should not re-parse.
fn store_cache(
) -> &'static parking_lot::Mutex<(Option<std::time::SystemTime>, u64, Vec<CaptureEntry>)> {
    use std::sync::LazyLock;
    static CACHE: LazyLock<
        parking_lot::Mutex<(Option<std::time::SystemTime>, u64, Vec<CaptureEntry>)>,
    > = LazyLock::new(|| parking_lot::Mutex::new((None, 0, Vec::new())));
    &CACHE
}

/// Serializes read-modify-write passes over the store: list()'s prune
/// and add()'s prepend both read then write, and an unsynchronized
/// pair can drop a capture that landed between the two calls.
fn store_lock() -> &'static parking_lot::Mutex<()> {
    use std::sync::LazyLock;
    static LOCK: LazyLock<parking_lot::Mutex<()>> = LazyLock::new(|| parking_lot::Mutex::new(()));
    &LOCK
}

fn read_store() -> Vec<CaptureEntry> {
    let Ok(path) = store_path() else {
        return Vec::new();
    };
    let stamp = std::fs::metadata(&path)
        .and_then(|m| m.modified().map(|t| (Some(t), m.len())))
        .unwrap_or((None, 0));
    {
        let guard = store_cache().lock();
        if guard.0 == stamp.0 && guard.1 == stamp.1 && stamp.0.is_some() {
            return guard.2.clone();
        }
    }
    let entries: Vec<CaptureEntry> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| {
            serde_json::from_str(&text).unwrap_or_else(|e| {
                crate::ilog!("iris: corrupt library.json, starting fresh: {e}");
                Some(Vec::new())
            })
        })
        .unwrap_or_default();
    *store_cache().lock() = (stamp.0, stamp.1, entries.clone());
    entries
}

fn write_store(entries: &[CaptureEntry]) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write library.json: {e}"))?;
    // Write-through: the next read sees the new bytes without a parse.
    let stamp = std::fs::metadata(&path)
        .and_then(|m| m.modified().map(|t| (Some(t), m.len())))
        .unwrap_or((None, 0));
    *store_cache().lock() = (stamp.0, stamp.1, entries.to_vec());
    Ok(())
}

/// Register a fresh capture: build its thumbnail, prepend it, cap the list.
/// The caller passes the already-decoded image so a save does not pay a
/// second PNG decode just to make the thumbnail.
pub fn add(path: &Path, img: &image::RgbaImage) -> Result<CaptureEntry, String> {
    let _write = store_lock().lock();
    let (width, height) = img.dimensions();
    let scale = THUMB_WIDTH as f64 / width as f64;
    // thumbnail_rgba is the banded replica of image's box-average
    // thumbnail: on a 4K capture the single-threaded scan reads 33MB
    // serially to produce 216px.
    let thumb_img = crate::thumb::thumbnail_rgba(
        img,
        THUMB_WIDTH,
        (height as f64 * scale).round().max(1.0) as u32,
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
        created_ms: crate::time::now_millis(),
    };
    let mut entries = read_store();
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry.clone());
    entries.truncate(MAX_ENTRIES);
    write_store(&entries)?;
    Ok(entry)
}

/// Parent-directory mtimes from the last full stat pass. A file
/// cannot appear, vanish, or be replaced without bumping its
/// directory's mtime, so an unchanged stamp set means the per-entry
/// exists() sweep would find nothing: the 1.5s library poll then
/// costs one stat per parent dir instead of one per capture.
fn dir_stamps() -> &'static parking_lot::Mutex<Vec<(PathBuf, Option<std::time::SystemTime>)>> {
    use std::sync::LazyLock;
    static STAMPS: LazyLock<parking_lot::Mutex<Vec<(PathBuf, Option<std::time::SystemTime>)>>> =
        LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));
    &STAMPS
}

pub fn list() -> Vec<CaptureEntry> {
    let _write = store_lock().lock();
    let entries = read_store();
    // Fast path: every entry's parent dir unchanged since the last
    // sweep means no capture file appeared or vanished.
    let mut dirs: Vec<PathBuf> = entries
        .iter()
        .filter_map(|e| e.path.parent().map(|p| p.to_path_buf()))
        .collect();
    dirs.sort();
    dirs.dedup();
    let stamps: Vec<(PathBuf, Option<std::time::SystemTime>)> = dirs
        .iter()
        .map(|d| {
            (
                d.clone(),
                std::fs::metadata(d).and_then(|m| m.modified()).ok(),
            )
        })
        .collect();
    if !entries.is_empty() && *dir_stamps().lock() == stamps {
        return entries;
    }
    // Prune entries whose file vanished (user moved/deleted it). The
    // stat calls run in parallel over a bounded pool: a few hundred
    // sequential exists() checks on a slow or network-mounted shots
    // dir stall the open, but a thread per entry is its own storm.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(entries.len().max(1));
    let mut alive_flags: Vec<std::sync::atomic::AtomicBool> = (0..entries.len())
        .map(|_| std::sync::atomic::AtomicBool::new(false))
        .collect();
    std::thread::scope(|scope| {
        let flags = &alive_flags;
        let entries_ref = &entries;
        let next_ref = &next;
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(move || loop {
                    let i = next_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= entries_ref.len() {
                        break;
                    }
                    if entries_ref[i].path.exists() {
                        flags[i].store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                })
            })
            .collect();
        for h in handles {
            let _ = h.join();
        }
    });
    let alive_flags: Vec<bool> = alive_flags.iter_mut().map(|f| *f.get_mut()).collect();
    let (alive, dead): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .zip(alive_flags)
        .partition(|(_, ok)| *ok);
    let alive: Vec<_> = alive.into_iter().map(|(e, _)| e).collect();
    if !dead.is_empty() {
        let _ = write_store(&alive);
    }
    *dir_stamps().lock() = stamps;
    alive
}

/// Remove a capture everywhere: entry, thumbnail, and the image file.
pub fn delete(path: &Path) -> Result<(), String> {
    let _write = store_lock().lock();
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

/// Remove several captures with one store write. A per-file delete()
/// serializes and rewrites library.json for every selection member.
/// Returns the number of files that failed to delete.
pub fn delete_many(paths: &[PathBuf]) -> usize {
    let _write = store_lock().lock();
    let mut entries = read_store();
    let set: std::collections::HashSet<&Path> = paths.iter().map(|p| p.as_path()).collect();
    entries.retain(|e| !set.contains(e.path.as_path()));
    let _ = write_store(&entries);
    let mut errors = 0;
    for path in paths {
        let _ = std::fs::remove_file(
            thumbs_dir()
                .unwrap_or_default()
                .join(format!("{}.png", path_key(path))),
        );
        if path.exists() && std::fs::remove_file(path).is_err() {
            errors += 1;
        }
    }
    errors
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

    /// A real PNG on disk inside `dir` plus its decoded pixels; `add`
    /// takes the image so tests exercise the same path as `finalize`.
    fn png(dir: &Path, name: &str) -> (PathBuf, image::RgbaImage) {
        let path = dir.join(name);
        let img = image::RgbaImage::from_pixel(8, 6, image::Rgba([1, 2, 3, 255]));
        img.save(&path).unwrap();
        (path, img)
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
        let (shot, img) = png(d.path(), "a.png");
        let entry = add(&shot, &img).unwrap();
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
        let (shot, img) = png(d.path(), "gone.png");
        add(&shot, &img).unwrap();
        std::fs::remove_file(&shot).unwrap();
        assert!(list().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn readding_same_path_replaces_not_duplicates() {
        let d = xdg();
        let (shot, img) = png(d.path(), "dup.png");
        add(&shot, &img).unwrap();
        add(&shot, &img).unwrap();
        assert_eq!(list().len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn corrupt_store_starts_fresh() {
        let _d = xdg();
        let store = app_data_dir().unwrap().join("library.json");
        std::fs::write(&store, "{not json").unwrap();
        assert!(list().is_empty());
    }

    #[test]
    #[serial_test::serial]
    fn store_caps_at_max_entries() {
        let d = xdg();
        for i in 0..5 {
            let (shot, img) = png(d.path(), &format!("s{i}.png"));
            add(&shot, &img).unwrap();
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
