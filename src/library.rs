use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Thumbnails fit inside this pixel box: the library card's 216x132
/// logical image area at 2x device scale, so a HiDPI display draws the
/// card from at least as many pixels as it shows.
pub const THUMB_BOX: (u32, u32) = (432, 264);

/// One capture in the library: the image plus its cached thumbnail.
#[derive(Serialize, Deserialize, Clone)]
pub struct CaptureEntry {
    pub path: PathBuf,
    pub thumb: PathBuf,
    pub width: u32,
    pub height: u32,
    pub created_ms: i64,
    /// When the editor last saved over the capture, or 0. A save keeps
    /// the capture's place and day in the library and changes this.
    #[serde(default)]
    pub edited_ms: i64,
}

impl CaptureEntry {
    /// The capture as last written: a new capture at the path or an
    /// editor save over it changes it, and with it the thumbnail.
    pub fn revision(&self) -> (i64, i64) {
        (self.created_ms, self.edited_ms)
    }
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

/// The store as this process last read or wrote it.
#[derive(Default)]
struct Cached {
    /// The file's mtime and length when it held `entries`; None until a
    /// read, while a write is in flight, and after a write failed.
    stamp: Option<(std::time::SystemTime, u64)>,
    entries: Vec<CaptureEntry>,
    /// A write of `entries` is in flight: they are newer than the file.
    writing: bool,
    /// Bumped by every write, so a read that parsed the file before a
    /// write began does not cache the older entries over it.
    version: u64,
}

/// The parsed store behind an mtime+len check: the library window
/// calls list() on every change it learns of, and an unchanged file
/// should not re-parse.
fn store_cache() -> &'static parking_lot::Mutex<Cached> {
    use std::sync::LazyLock;
    static CACHE: LazyLock<parking_lot::Mutex<Cached>> = LazyLock::new(Default::default);
    &CACHE
}

/// Serializes read-modify-write passes over the store: list()'s prune,
/// add()'s prepend, a trash, and a thumbnail rebuild each read then
/// write, and an unsynchronized pair can drop a capture that landed
/// between the two calls. A read alone takes no lock.
fn store_lock() -> &'static parking_lot::Mutex<()> {
    use std::sync::LazyLock;
    static LOCK: LazyLock<parking_lot::Mutex<()>> = LazyLock::new(|| parking_lot::Mutex::new(()));
    &LOCK
}

/// Runs at every store write in this process, once the new entries are
/// readable and before their bytes reach the disk, and again when that
/// disk write fails. The process that shows the library sets it, so an
/// open library window reads a write at once.
static ON_WRITE: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Run `hook` at each store write in this process. The first call sets
/// it for the process lifetime.
pub fn on_write(hook: fn()) {
    let _ = ON_WRITE.set(hook);
}

fn run_write_hook() {
    if let Some(hook) = ON_WRITE.get() {
        hook();
    }
}

fn file_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn read_store() -> Vec<CaptureEntry> {
    let Ok(path) = store_path() else {
        return Vec::new();
    };
    let version = {
        let cached = store_cache().lock();
        if cached.writing {
            return cached.entries.clone();
        }
        cached.version
    };
    let stamp = file_stamp(&path);
    {
        let cached = store_cache().lock();
        if cached.writing || (stamp.is_some() && cached.stamp == stamp) {
            return cached.entries.clone();
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
    let mut cached = store_cache().lock();
    if !cached.writing && cached.version == version {
        cached.stamp = stamp;
        cached.entries = entries.clone();
    }
    entries
}

/// Write `entries` as the store. Readers in this process get them from
/// the moment the write starts: a rename over a file can wait tens of
/// milliseconds on the filesystem journal, and an open library window
/// shows a new capture without waiting on it. A failed write leaves the
/// file as it was, and the next read parses it again. The caller holds
/// [`store_lock`].
fn write_store(entries: Vec<CaptureEntry>) -> Result<(), String> {
    let path = store_path()?;
    let text = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    {
        let mut cached = store_cache().lock();
        cached.entries = entries;
        cached.stamp = None;
        cached.writing = true;
        cached.version += 1;
    }
    run_write_hook();
    let written = crate::output::replace(&path, text.as_bytes());
    {
        let mut cached = store_cache().lock();
        cached.writing = false;
        cached.stamp = written.as_ref().ok().and_then(|()| file_stamp(&path));
    }
    if let Err(e) = written {
        // The window read entries the file does not hold: read again.
        run_write_hook();
        return Err(format!("write library.json: {e}"));
    }
    Ok(())
}

/// The size of a `w`x`h` capture's thumbnail: the whole capture scaled
/// to fit inside THUMB_BOX, its aspect kept, never enlarged. The card
/// shows all of it.
pub fn thumb_size(w: u32, h: u32) -> (u32, u32) {
    let (bw, bh) = (f64::from(THUMB_BOX.0), f64::from(THUMB_BOX.1));
    let (w, h) = (f64::from(w.max(1)), f64::from(h.max(1)));
    let scale = (bw / w).min(bh / h).min(1.0);
    (
        (w * scale).round().max(1.0) as u32,
        (h * scale).round().max(1.0) as u32,
    )
}

/// Box-filter `img` down to its thumbnail size.
fn make_thumb(img: &image::RgbaImage) -> image::RgbaImage {
    let (tw, th) = thumb_size(img.width(), img.height());
    if (tw, th) == img.dimensions() {
        return img.clone();
    }
    // The banded replica of image's box-average thumbnail: a 4K
    // capture is a 33MB scan, split across cores.
    crate::thumb::thumbnail_rgba(img, tw, th)
}

/// Register a fresh capture: build its thumbnail and list it first. The
/// library lists every capture whose file exists; nothing drops one for
/// age or count.
/// The caller passes the already-decoded image so a save does not pay a
/// second PNG decode just to make the thumbnail.
pub fn add(path: &Path, img: &image::RgbaImage) -> Result<CaptureEntry, String> {
    let _write = store_lock().lock();
    record(path, img, false)
}

/// [`add`] on a thread of its own, for a caller that shows the capture
/// without waiting on the thumbnail and the store write. Returns once
/// that thread holds the store lock, so every store change this process
/// starts after the return lands after the add. `failed` receives the
/// reason a started add failed.
pub fn add_detached(
    path: PathBuf,
    img: std::sync::Arc<image::RgbaImage>,
    failed: impl FnOnce(String) + Send + 'static,
) -> Result<(), String> {
    let (held, holds) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("iris-library".into())
        .spawn(move || {
            let _write = store_lock().lock();
            let _ = held.send(());
            if let Err(e) = record(&path, &img, false) {
                failed(e);
            }
        })
        .map_err(|e| format!("spawn library thread: {e}"))?;
    holds
        .recv()
        .map_err(|_| "the library thread ended before it took the store".to_string())
}

/// Record an editor save over `path`: a fresh thumbnail, the image's
/// size, and a new `edited_ms`, with the entry kept where it is. A path
/// the library does not list (the editor opens any PNG) is added as a
/// fresh capture.
pub fn update(path: &Path, img: &image::RgbaImage) -> Result<CaptureEntry, String> {
    let _write = store_lock().lock();
    record(path, img, true)
}

/// [`add`] or [`update`]; the caller holds [`store_lock`].
fn record(path: &Path, img: &image::RgbaImage, in_place: bool) -> Result<CaptureEntry, String> {
    let (width, height) = img.dimensions();
    let thumb = thumbs_dir()?.join(format!("{}.png", path_key(path)));
    make_thumb(img)
        .save(&thumb)
        .map_err(|e| format!("save thumbnail: {e}"))?;

    let now = crate::time::now_millis();
    let mut entries = read_store();
    let listed = entries
        .iter_mut()
        .find(|e| e.path == path)
        .filter(|_| in_place);
    let entry = if let Some(e) = listed {
        (e.thumb, e.width, e.height) = (thumb, width, height);
        // A new revision within one millisecond and after the clock
        // went back.
        e.edited_ms = now.max(e.edited_ms + 1);
        e.clone()
    } else {
        let entry = CaptureEntry {
            path: path.to_path_buf(),
            thumb,
            width,
            height,
            created_ms: now,
            edited_ms: 0,
        };
        entries.retain(|e| e.path != entry.path);
        entries.insert(0, entry.clone());
        entry
    };
    write_store(entries)?;
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
/// its own save. A failed write still returns the pixels.
fn rebuild_thumb(entry: &CaptureEntry) -> Result<image::RgbaImage, String> {
    let src = image::open(&entry.path)
        .map_err(|e| format!("cannot open {}: {e}", entry.path.display()))?
        .into_rgba8();
    let thumb = make_thumb(&src);
    let _write = store_lock().lock();
    let mut entries = read_store();
    let Some(stored) = entries
        .iter_mut()
        .find(|e| e.path == entry.path && e.revision() == entry.revision())
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
        if let Err(e) = write_store(entries) {
            crate::ilog!("iris: {e}");
        }
    }
    Ok(thumb)
}

/// Parent-directory mtimes from the last full stat pass. A file
/// cannot appear, vanish, or be replaced without bumping its
/// directory's mtime, so an unchanged stamp set means the per-entry
/// exists() sweep would find nothing: a library pass then costs one
/// stat per parent dir instead of one per capture.
type DirStamps = parking_lot::Mutex<Vec<(PathBuf, Option<std::time::SystemTime>)>>;

fn dir_stamps() -> &'static DirStamps {
    use std::sync::LazyLock;
    static STAMPS: LazyLock<DirStamps> = LazyLock::new(|| parking_lot::Mutex::new(Vec::new()));
    &STAMPS
}

pub fn list() -> Vec<CaptureEntry> {
    let entries = read_store();
    // Fast path: every entry's parent dir unchanged since the last
    // sweep means no capture file appeared or vanished.
    let dirs: std::collections::HashSet<&Path> =
        entries.iter().filter_map(|e| e.path.parent()).collect();
    let mut stamps: Vec<(PathBuf, Option<std::time::SystemTime>)> = dirs
        .into_iter()
        .map(|d| {
            (
                d.to_path_buf(),
                std::fs::metadata(d).and_then(|m| m.modified()).ok(),
            )
        })
        .collect();
    stamps.sort_unstable_by(|a, b| a.0.cmp(&b.0));
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
    let alive_flags: Vec<std::sync::atomic::AtomicBool> = (0..entries.len())
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
    let dead: std::collections::HashSet<&Path> = entries
        .iter()
        .zip(&alive_flags)
        .filter(|(_, alive)| !alive.load(std::sync::atomic::Ordering::Relaxed))
        .map(|(e, _)| e.path.as_path())
        .collect();
    let listed = if dead.is_empty() {
        drop(dead);
        entries
    } else {
        prune(&dead)
    };
    // Stamps read before the sweep: a change since then reads as a
    // changed directory on the next list, which sweeps again.
    *dir_stamps().lock() = stamps;
    listed
}

/// Drop the entries of `dead` from the store as it is now: a capture
/// recorded since the sweep keeps its entry, and so does one whose file
/// was saved again since.
fn prune(dead: &std::collections::HashSet<&Path>) -> Vec<CaptureEntry> {
    let _write = store_lock().lock();
    let mut entries = read_store();
    let before = entries.len();
    entries.retain(|e| !dead.contains(e.path.as_path()) || e.path.exists());
    if entries.len() != before {
        let _ = write_store(entries.clone());
    }
    entries
}

/// What the platform calls its trash, for messages and menu items.
pub const TRASH_NAME: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "Trash"
};

/// Move a capture to the system trash, then drop its library entry and
/// thumbnail. A path the library does not list is trashed all the same
/// (the editor opens any PNG). A file that is already gone drops its
/// entry. A failed move leaves the file and its entry in place and
/// returns the reason.
pub fn trash(path: &Path) -> Result<(), String> {
    match trash_many(std::slice::from_ref(&path.to_path_buf())).pop() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// [`trash`] for several captures with one store write. Returns one
/// message per capture that stayed, in `paths` order; the rest left the
/// folder and the library.
pub fn trash_many(paths: &[PathBuf]) -> Vec<String> {
    // The system move runs outside the store lock: a cross-device copy
    // into the trash can take seconds, and list() must not wait on it.
    let moved = move_to_trash(paths);
    let mut errors = Vec::new();
    let mut gone = std::collections::HashSet::new();
    for (path, outcome) in paths.iter().zip(moved) {
        match outcome {
            Ok(()) => {
                gone.insert(path.as_path());
            }
            Err(e) => errors.push(e),
        }
    }
    if gone.is_empty() {
        return errors;
    }
    let _write = store_lock().lock();
    let (dropped, kept): (Vec<CaptureEntry>, Vec<CaptureEntry>) = read_store()
        .into_iter()
        .partition(|e| gone.contains(e.path.as_path()));
    if dropped.is_empty() {
        return errors;
    }
    if let Err(e) = write_store(kept) {
        errors.push(e);
    }
    for entry in dropped {
        let _ = std::fs::remove_file(&entry.thumb);
    }
    errors
}

/// The system trash move of each of `paths`, in order. A path that no
/// longer exists counts as moved.
fn move_to_trash(paths: &[PathBuf]) -> Vec<Result<(), String>> {
    let each = || -> Vec<Result<(), String>> { paths.iter().map(|p| move_one(p)).collect() };
    // The Recycle Bin call initializes COM on its thread for the life of
    // the thread and panics when the thread already runs another COM
    // apartment model: a scoped thread of its own contains both.
    if !cfg!(windows) {
        return each();
    }
    std::thread::scope(|s| s.spawn(each).join()).unwrap_or_else(|_| {
        paths
            .iter()
            .map(|p| Err(trash_error(p, "the shell file operation failed")))
            .collect()
    })
}

fn move_one(path: &Path) -> Result<(), String> {
    match path.try_exists() {
        Ok(false) => return Ok(()),
        Ok(true) => {}
        Err(e) => return Err(trash_error(path, &e.to_string())),
    }
    trash_context()
        .delete(path)
        .map_err(|e| trash_error(path, &trash_reason(e)))
}

/// The OS trash call. On macOS it runs through NSFileManager: the
/// crate's default Finder route drives Finder over AppleScript, which
/// prompts for Automation access on the first move and plays the Finder
/// sound on every one.
fn trash_context() -> ::trash::TrashContext {
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut ctx = ::trash::TrashContext::new();
    #[cfg(target_os = "macos")]
    {
        use ::trash::macos::{DeleteMethod, TrashContextExtMacos};
        ctx.set_delete_method(DeleteMethod::NsFileManager);
    }
    ctx
}

fn trash_error(path: &Path, reason: &str) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("Could not move {name} to the {TRASH_NAME}: {reason}")
}

/// A `trash` crate error as a sentence; its Display is a Debug dump.
fn trash_reason(e: ::trash::Error) -> String {
    use ::trash::Error;
    match e {
        Error::Unknown { description } => description,
        Error::Os { code, description } => format!("{description} (code {code})"),
        #[cfg(all(
            unix,
            not(target_os = "macos"),
            not(target_os = "ios"),
            not(target_os = "android")
        ))]
        Error::FileSystem { path, source } => format!("{source}: {}", path.display()),
        Error::TargetedRoot => "the path is a root folder".to_owned(),
        Error::CouldNotAccess { target } => format!("cannot access {target}"),
        Error::CanonicalizePath { original } => format!("cannot resolve {}", original.display()),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests;
