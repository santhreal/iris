use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 200;
/// Thumbnails cover this pixel box: the library card's 216x132 logical
/// image area at 2x device scale, so a HiDPI display draws the card
/// from at least as many pixels as it shows.
const THUMB_BOX: (u32, u32) = (432, 264);

/// One capture in the library: the image plus its cached thumbnail.
#[derive(Serialize, Deserialize, Clone)]
pub struct CaptureEntry {
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub width: u32,
    pub height: u32,
    pub created_ms: i64,
}

/// The shared data directory, created on first use.
pub fn app_data_dir() -> Result<PathBuf, String> {
    let dir = crate::dirs::data_dir().ok_or("no project data dir")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app data dir: {e}"))?;
    Ok(dir)
}

/// The shared cache directory (thumbnails, frozen frames).
pub fn app_cache_dir() -> Result<PathBuf, String> {
    let dir = crate::dirs::cache_dir().ok_or("no project cache dir")?;
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

/// mtime, length, and parsed entries of the store file.
type StoreCache = parking_lot::Mutex<(Option<std::time::SystemTime>, u64, Vec<CaptureEntry>)>;

/// The parsed store behind an mtime+len check: the library window
/// polls list() every 1.5s, and an unchanged file should not re-parse.
fn store_cache() -> &'static StoreCache {
    use std::sync::LazyLock;
    static CACHE: LazyLock<StoreCache> =
        LazyLock::new(|| parking_lot::Mutex::new((None, 0, Vec::new())));
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

/// A `w`x`h` capture's thumbnail plan: the size it scales to, covering
/// THUMB_BOX and never enlarged, then the size of the crop to the box's
/// aspect around its center. The card shows that center crop.
fn thumb_plan(w: u32, h: u32) -> ((u32, u32), (u32, u32)) {
    let (bw, bh) = (f64::from(THUMB_BOX.0), f64::from(THUMB_BOX.1));
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let scale = (bw / w).max(bh / h).min(1.0);
    let (sw, sh) = ((w * scale).round().max(1.0), (h * scale).round().max(1.0));
    let aspect = bw / bh;
    let (cw, ch) = (
        sw.min((sh * aspect).round()).max(1.0),
        sh.min((sw / aspect).round()).max(1.0),
    );
    ((sw as u32, sh as u32), (cw as u32, ch as u32))
}

/// The size of a `w`x`h` capture's thumbnail.
fn thumb_size(w: u32, h: u32) -> (u32, u32) {
    thumb_plan(w, h).1
}

/// Box-filter `img` down to its thumbnail plan and crop the center.
fn make_thumb(img: &image::RgbaImage) -> image::RgbaImage {
    let ((sw, sh), (cw, ch)) = thumb_plan(img.width(), img.height());
    let scaled;
    let base = if (sw, sh) == img.dimensions() {
        img
    } else {
        // The banded replica of image's box-average thumbnail: a 4K
        // capture is a 33MB scan, split across cores.
        scaled = crate::thumb::thumbnail_rgba(img, sw, sh);
        &scaled
    };
    image::imageops::crop_imm(base, (sw - cw) / 2, (sh - ch) / 2, cw, ch).to_image()
}

/// Register a fresh capture: build its thumbnail, prepend it, cap the list.
/// The caller passes the already-decoded image so a save does not pay a
/// second PNG decode just to make the thumbnail.
pub fn add(path: &Path, img: &image::RgbaImage) -> Result<CaptureEntry, String> {
    let _write = store_lock().lock();
    let (width, height) = img.dimensions();
    let thumb = thumbs_dir()?.join(format!("{}.png", path_key(path)));
    make_thumb(img)
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

/// The entry's thumbnail pixels. A cached file that is missing,
/// unreadable, or of another size (an older build's sizing) is rebuilt
/// from the capture and written back. When the capture cannot be read
/// either, an old thumbnail still shows it.
pub fn thumbnail(entry: &CaptureEntry) -> Result<image::RgbaImage, String> {
    let cached = image::open(&entry.thumb)
        .ok()
        .map(image::DynamicImage::into_rgba8);
    match cached {
        Some(img) if img.dimensions() == thumb_size(entry.width, entry.height) => Ok(img),
        cached => rebuild_thumb(entry).or_else(|e| cached.ok_or(e)),
    }
}

/// Rebuild `entry`'s thumbnail from its capture. The file is written
/// back only while the store still holds this exact entry: an entry
/// that changed meanwhile (an editor save) got a fresh thumbnail from
/// its own add. A failed write still returns the pixels.
fn rebuild_thumb(entry: &CaptureEntry) -> Result<image::RgbaImage, String> {
    let src = image::open(&entry.path)
        .map_err(|e| format!("cannot open {}: {e}", entry.path.display()))?
        .into_rgba8();
    let thumb = make_thumb(&src);
    let _write = store_lock().lock();
    let mut entries = read_store();
    let Some(stored) = entries
        .iter_mut()
        .find(|e| e.path == entry.path && e.created_ms == entry.created_ms)
    else {
        return Ok(thumb);
    };
    let saved = entry
        .thumb
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .map_err(|e| e.to_string())
        .and_then(|()| thumb.save(&entry.thumb).map_err(|e| e.to_string()));
    if let Err(e) = saved {
        crate::ilog!("iris: save thumbnail {}: {e}", entry.thumb.display());
    }
    // A capture resized outside iris: the entry takes its real size, or
    // every load would find the thumbnail stale and rebuild it again.
    if src.dimensions() != (stored.width, stored.height) {
        (stored.width, stored.height) = src.dimensions();
        if let Err(e) = write_store(&entries) {
            crate::ilog!("iris: {e}");
        }
    }
    Ok(thumb)
}

/// Parent-directory mtimes from the last full stat pass. A file
/// cannot appear, vanish, or be replaced without bumping its
/// directory's mtime, so an unchanged stamp set means the per-entry
/// exists() sweep would find nothing: the 1.5s library poll then
/// costs one stat per parent dir instead of one per capture.
type DirStamps = parking_lot::Mutex<Vec<(PathBuf, Option<std::time::SystemTime>)>>;

fn dir_stamps() -> &'static DirStamps {
    use std::sync::LazyLock;
    static STAMPS: LazyLock<DirStamps> = LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));
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

#[cfg(test)]
mod tests;
