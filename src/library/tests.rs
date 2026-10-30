// WHY: the class closed here is "the library loses or corrupts captures":
// a store that does not round-trip, a trash that leaves the file or loses
// it outright, a cap that drops the oldest captures, a store a reader
// finds half written, or a path_key that collides all surface as
// missing thumbnails or vanished shots; a
// thumbnail smaller than the card's 2x box, or one kept at an older
// build's size, shows as a soft card on a HiDPI display. Env-mutating
// tests run serially with IRIS_HOME (and the freedesktop trash) pointed
// at a tempdir. Not covered: the library window's decode of the returned
// pixels.
use super::*;

/// Root iris data/cache in a fresh tempdir via `IRIS_HOME` (on every
/// platform); returns it for file seeds. The freedesktop home trash
/// (`$XDG_DATA_HOME/Trash`) is in the same tempdir, so a trash in a test
/// never reaches the trash of the account running it.
fn isolated_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(crate::dirs::HOME_ENV, dir.path());
    std::env::set_var("XDG_DATA_HOME", dir.path().join("xdg"));
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

// WHY: a delete from the library moves the capture to the system trash,
// where it can be restored, instead of unlinking it. Closed here: a trash
// that unlinks or leaves the file, a moved capture that keeps its entry
// or thumbnail, a failed move that drops the entry of a file still on
// disk (the capture would vanish from the library and stay in the
// folder), and a capture already gone that keeps an entry nothing can
// remove. Not covered: the macOS and Windows trash calls, which the
// `trash` crate's own tests exercise.
#[test]
#[serial_test::serial]
fn add_list_trash_round_trips() {
    let d = isolated_home();
    let (shot, img) = png(d.path(), "a.png");
    let entry = add(&shot, &img).unwrap();
    assert!(entry.thumb.exists());
    assert_eq!(list().len(), 1);
    trash(&shot).unwrap();
    assert!(list().is_empty());
    assert!(!entry.thumb.exists());
    assert!(!shot.exists());
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let trashed = d.path().join("xdg/Trash/files/a.png");
        assert!(trashed.exists(), "the capture is in the trash");
        assert!(d.path().join("xdg/Trash/info/a.png.trashinfo").exists());
    }
}

#[test]
#[serial_test::serial]
fn trash_of_a_capture_already_gone_drops_its_entry() {
    let d = isolated_home();
    let (shot, img) = png(d.path(), "gone.png");
    let entry = add(&shot, &img).unwrap();
    std::fs::remove_file(&shot).unwrap();
    assert_eq!(trash(&shot), Ok(()));
    assert!(read_store().is_empty());
    assert!(!entry.thumb.exists());
    // A path the store never listed and the disk never held.
    assert_eq!(trash(Path::new("/nonexistent/never.png")), Ok(()));
}

#[test]
#[serial_test::serial]
#[cfg(all(unix, not(target_os = "macos")))]
fn a_failed_trash_keeps_the_file_and_its_entry() {
    let d = isolated_home();
    let (a, img) = png(d.path(), "a.png");
    let (b, _) = png(d.path(), "b.png");
    let ea = add(&a, &img).unwrap();
    let eb = add(&b, &img).unwrap();
    // A file where the trash directory goes: the move has nowhere to go.
    std::fs::create_dir_all(d.path().join("xdg")).unwrap();
    std::fs::write(d.path().join("xdg/Trash"), b"").unwrap();
    let err = trash(&a).unwrap_err();
    assert!(
        err.starts_with(&format!("Could not move a.png to the {TRASH_NAME}: ")),
        "{err}"
    );
    let errors = trash_many(&[b.clone(), a.clone()]);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors[0].starts_with("Could not move b.png"), "{errors:?}");
    assert!(errors[1].starts_with("Could not move a.png"), "{errors:?}");
    for (shot, entry) in [(&a, &ea), (&b, &eb)] {
        assert!(shot.exists(), "{} stays on disk", shot.display());
        assert!(entry.thumb.exists());
    }
    let stored: Vec<_> = read_store().into_iter().map(|e| e.path).collect();
    assert_eq!(stored.len(), 2);
    assert!(stored.contains(&a) && stored.contains(&b));
    // The trash comes back: the same captures move, and only they leave.
    std::fs::remove_file(d.path().join("xdg/Trash")).unwrap();
    assert!(trash_many(std::slice::from_ref(&a)).is_empty());
    assert!(!a.exists() && b.exists());
    let stored: Vec<_> = read_store().into_iter().map(|e| e.path).collect();
    assert_eq!(stored, [b]);
}

#[test]
#[serial_test::serial]
fn list_prunes_entries_whose_file_vanished() {
    let d = isolated_home();
    let (shot, img) = png(d.path(), "gone.png");
    add(&shot, &img).unwrap();
    std::fs::remove_file(&shot).unwrap();
    assert!(list().is_empty());
}

#[test]
#[serial_test::serial]
fn readding_same_path_replaces_not_duplicates() {
    let d = isolated_home();
    let (shot, img) = png(d.path(), "dup.png");
    add(&shot, &img).unwrap();
    add(&shot, &img).unwrap();
    assert_eq!(list().len(), 1);
}

#[test]
#[serial_test::serial]
fn corrupt_store_starts_fresh() {
    let _d = isolated_home();
    let store = app_data_dir().unwrap().join("library.json");
    std::fs::write(&store, "{not json").unwrap();
    assert!(list().is_empty());
}

/// A capture's entry for `name` in `dir`, taken at `ms`, with no file.
fn listed(dir: &Path, name: &str, ms: i64) -> CaptureEntry {
    CaptureEntry {
        path: dir.join(name),
        thumb: PathBuf::new(),
        width: 1,
        height: 1,
        created_ms: ms,
        edited_ms: 0,
    }
}

#[test]
#[serial_test::serial]
fn a_new_capture_keeps_every_older_one_listed() {
    let d = isolated_home();
    // Seeded directly: a thousand adds of real PNGs is slow.
    let older: Vec<CaptureEntry> = (0..1000)
        .map(|i| listed(d.path(), &format!("old{i}.png"), 1000 - i))
        .collect();
    write_store(older.clone()).unwrap();
    let (shot, img) = png(d.path(), "new.png");
    add(&shot, &img).unwrap();
    let stored: Vec<PathBuf> = read_store().into_iter().map(|e| e.path).collect();
    assert_eq!(stored.len(), 1001);
    assert_eq!(stored[0], shot);
    let paths: Vec<PathBuf> = older.into_iter().map(|e| e.path).collect();
    assert_eq!(stored[1..], paths[..]);
}

#[test]
#[serial_test::serial]
#[cfg(unix)]
fn a_reader_never_finds_the_store_half_written() {
    let d = isolated_home();
    let store = store_path().unwrap();
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (done, reads) = (AtomicBool::new(false), AtomicUsize::new(0));
    std::thread::scope(|s| {
        let reader = s.spawn(|| {
            while !done.load(Ordering::Acquire) {
                let Ok(bytes) = std::fs::read(&store) else {
                    continue;
                };
                let parsed = serde_json::from_slice::<Vec<CaptureEntry>>(&bytes);
                assert!(parsed.is_ok(), "a torn read of {} bytes", bytes.len());
                reads.fetch_add(1, Ordering::Release);
            }
        });
        // Stores large enough that a write takes many reads' time, until
        // the reader has read a few, and at most 400.
        let wrote = (0..400)
            .take_while(|&round| round < 40 || reads.load(Ordering::Acquire) < 10)
            .try_for_each(|round| {
                let entries: Vec<CaptureEntry> = (0..2000 + round)
                    .map(|i| listed(d.path(), &format!("s{round}-{i}.png"), i))
                    .collect();
                write_store(entries)
            });
        // The reader stops before a writer's error is raised: a scope
        // joins every thread, and a reader left running never ends.
        done.store(true, Ordering::Release);
        let read = reader.join();
        wrote.unwrap();
        read.expect("reader");
    });
    assert!(reads.into_inner() >= 10, "the reader read too little");
    let names: Vec<_> = std::fs::read_dir(store.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["library.json"]);
}

/// A `w`x`h` PNG of one color at `path`.
fn solid(path: &Path, w: u32, h: u32, color: [u8; 4]) -> image::RgbaImage {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
    img.save(path).unwrap();
    img
}

const GREEN: [u8; 4] = [0, 200, 0, 255];
const RED: [u8; 4] = [200, 0, 0, 255];

// WHY: closes "the library card shows part of a capture": a
// thumbnail holds the whole capture, scaled to fit the card box with
// its aspect kept, and never enlarged. It does not cover how the card
// draws the thumbnail; card.rs's tests do.
#[test]
fn thumb_size_fits_the_whole_capture_in_the_box() {
    let cases = [
        ((1920, 1080), (432, 243)),
        ((3840, 2160), (432, 243)),
        ((1080, 1920), (149, 264)),
        ((432, 264), (432, 264)),
        ((864, 528), (432, 264)),
        // Smaller than the box on one side: scaled to fit the other.
        ((300, 1000), (79, 264)),
        ((1000, 200), (432, 86)),
        // Smaller than the box: never enlarged.
        ((8, 6), (8, 6)),
        ((400, 100), (400, 100)),
        // Extreme strips keep at least one pixel.
        ((100_000, 10), (432, 1)),
        ((0, 0), (1, 1)),
    ];
    for ((w, h), fitted) in cases {
        let (tw, th) = thumb_size(w, h);
        assert_eq!((tw, th), fitted, "{w}x{h}");
        assert!(
            tw <= THUMB_BOX.0 && th <= THUMB_BOX.1,
            "{w}x{h} fits the box"
        );
    }
}

#[test]
#[serial_test::serial]
fn add_writes_the_whole_capture_at_the_fitted_size() {
    let d = isolated_home();
    // Red bands at both edges: a thumbnail of the whole capture shows
    // them, where a center crop cut them off.
    let mut img = image::RgbaImage::from_pixel(880, 495, image::Rgba(GREEN));
    for y in 0..495 {
        for x in (0..20).chain(860..880) {
            img.put_pixel(x, y, image::Rgba(RED));
        }
    }
    let shot = d.path().join("wide.png");
    img.save(&shot).unwrap();
    let entry = add(&shot, &img).unwrap();
    let thumb = image::open(&entry.thumb).unwrap().into_rgba8();
    assert_eq!(thumb.dimensions(), (432, 243));
    for y in [0, 121, 242] {
        assert_eq!(thumb.get_pixel(0, y).0, RED, "left edge, row {y}");
        assert_eq!(thumb.get_pixel(431, y).0, RED, "right edge, row {y}");
        assert_eq!(thumb.get_pixel(216, y).0, GREEN, "center, row {y}");
    }
    assert_eq!(thumbnail(&entry).unwrap(), thumb);
}

#[test]
#[serial_test::serial]
fn thumbnail_reads_a_current_file_without_rebuilding() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    // The right size in another color: served from the file, so the
    // capture is not decoded again.
    solid(&entry.thumb, 432, 243, RED);
    assert_eq!(thumbnail(&entry).unwrap().get_pixel(0, 0).0, RED);
}

#[test]
#[serial_test::serial]
fn thumbnail_rebuilds_a_stale_or_missing_file() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    // An older build's 320px-wide thumbnail, an older build's center
    // crop to the box's aspect, then no file at all.
    for what in ["stale", "cropped", "missing"] {
        match what {
            "stale" => drop(solid(&entry.thumb, 320, 180, RED)),
            "cropped" => drop(solid(&entry.thumb, 432, 264, RED)),
            _ => std::fs::remove_file(&entry.thumb).unwrap(),
        }
        let rebuilt = thumbnail(&entry).unwrap();
        assert_eq!(rebuilt.dimensions(), (432, 243), "{what}");
        assert_eq!(rebuilt.get_pixel(0, 0).0, GREEN, "{what}");
        let on_disk = image::open(&entry.thumb).unwrap().into_rgba8();
        assert_eq!(on_disk, rebuilt, "{what}: written back");
    }
}

#[test]
#[serial_test::serial]
fn thumbnail_keeps_a_stale_file_when_the_capture_is_unreadable() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    solid(&entry.thumb, 320, 180, RED);
    std::fs::remove_file(&shot).unwrap();
    let kept = thumbnail(&entry).unwrap();
    assert_eq!(
        (kept.dimensions(), kept.get_pixel(0, 0).0),
        ((320, 180), RED)
    );
    std::fs::remove_file(&entry.thumb).unwrap();
    assert!(thumbnail(&entry).unwrap_err().contains("s.png"));
}

#[test]
#[serial_test::serial]
fn rebuild_leaves_the_file_of_an_entry_that_changed() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    solid(&entry.thumb, 320, 180, RED);
    // A snapshot from before an editor save re-added the capture.
    let older = CaptureEntry {
        created_ms: entry.created_ms - 1,
        ..entry.clone()
    };
    assert_eq!(thumbnail(&older).unwrap().dimensions(), (432, 243));
    let on_disk = image::open(&entry.thumb).unwrap().into_rgba8();
    assert_eq!(on_disk.dimensions(), (320, 180), "the newer entry's file");
}

#[test]
#[serial_test::serial]
fn rebuild_adopts_a_capture_resized_outside_iris() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    solid(&shot, 300, 300, GREEN);
    std::fs::remove_file(&entry.thumb).unwrap();
    assert_eq!(thumbnail(&entry).unwrap().dimensions(), (264, 264));
    let stored = list().into_iter().find(|e| e.path == shot).unwrap();
    assert_eq!((stored.width, stored.height), (300, 300));
    // The entry's size now matches its thumbnail: no rebuild next time.
    solid(&entry.thumb, 264, 264, RED);
    assert_eq!(thumbnail(&stored).unwrap().get_pixel(0, 0).0, RED);
}

static WRITES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What the write hook saw, when a test asks: per run, the paths list()
/// returned (None when it did not return within 5 seconds) and the paths
/// the store file held.
type Seen = Vec<(Option<Vec<PathBuf>>, Vec<PathBuf>)>;
static PROBE: parking_lot::Mutex<Option<Seen>> = parking_lot::Mutex::new(None);

fn count_write() {
    WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut probe = PROBE.lock();
    let Some(seen) = probe.as_mut() else {
        return;
    };
    // On a thread of its own, as a library window lists: a list that
    // waits on the writer shows as a timeout, not a hang.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(list().into_iter().map(|e| e.path).collect());
    });
    let on_disk: Vec<CaptureEntry> = std::fs::read(store_path().unwrap())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    seen.push((
        rx.recv_timeout(std::time::Duration::from_secs(5)).ok(),
        on_disk.into_iter().map(|e| e.path).collect(),
    ));
}

/// How many times `op` runs the store write hook.
fn hooked<T>(op: impl FnOnce() -> T) -> usize {
    on_write(count_write);
    let before = WRITES.load(std::sync::atomic::Ordering::SeqCst);
    op();
    WRITES.load(std::sync::atomic::Ordering::SeqCst) - before
}

// WHY: an open library window lists a capture the moment this process
// stores it, through the hook every store write runs. Closed here: a
// writer that changes the store without running the hook (the window
// shows the change only when something else makes it read the store),
// and a read that runs it (each list pass would queue another). Not
// covered: writes by another process, which a window shows at its next
// read of the store.
#[test]
#[serial_test::serial]
fn every_store_write_runs_the_write_hook_once() {
    let d = isolated_home();
    let (a, img) = png(d.path(), "a.png");
    let (b, _) = png(d.path(), "b.png");
    let (c, _) = png(d.path(), "c.png");
    let mut entry = None;
    assert_eq!(hooked(|| add(&a, &img)), 1, "add");
    assert_eq!(hooked(|| (add(&b, &img), add(&c, &img))), 2, "adds");
    assert_eq!(hooked(|| entry = add(&a, &img).ok()), 1, "re-add");
    let entry = entry.unwrap();
    // The first list in this directory sweeps it; a later one may take
    // the unchanged-directory path, which misses a removal made in the
    // same timestamp tick as the directory's last change.
    std::fs::remove_file(&c).unwrap();
    assert_eq!(hooked(list), 1, "prune");
    assert_eq!(hooked(list), 0, "list");
    assert_eq!(hooked(|| thumbnail(&entry)), 0, "current thumbnail");
    solid(&a, 300, 300, GREEN);
    std::fs::remove_file(&entry.thumb).unwrap();
    assert_eq!(hooked(|| thumbnail(&entry)), 1, "resized capture");
    assert_eq!(hooked(|| trash(&a)), 1, "trash");
    assert_eq!(hooked(|| trash(&a)), 0, "trash of an unstored path");
    assert_eq!(hooked(|| trash_many(&[b])), 1, "trash_many");
    assert!(list().is_empty());
}

// WHY: a library window lists a new capture when the write hook runs,
// and a rename over library.json can wait tens of milliseconds on the
// filesystem journal. Closed here: a hook that runs only after the disk
// write (the window waits on the journal), a list() that waits on the
// writer's lock, and a read during the write that returns the store
// file's older entries (the window shows the old listing and nothing
// makes it read again). Not covered: a reader in another process, which
// reads the file.
#[test]
#[serial_test::serial]
fn a_write_is_listed_while_its_bytes_are_in_flight() {
    let d = isolated_home();
    let (a, img) = png(d.path(), "a.png");
    let (b, _) = png(d.path(), "b.png");
    add(&a, &img).unwrap();
    on_write(count_write);
    *PROBE.lock() = Some(Vec::new());
    let added = add(&b, &img);
    let seen = PROBE.lock().take().unwrap();
    added.unwrap();
    assert_eq!(
        seen,
        [(Some(vec![b.clone(), a.clone()]), vec![a.clone()])],
        "what the hook listed, and what the store file held"
    );
    assert_eq!(stored(), [b, a]);
}

/// The paths the store file holds, newest first.
fn stored() -> Vec<PathBuf> {
    let bytes = std::fs::read(store_path().unwrap()).unwrap();
    serde_json::from_slice::<Vec<CaptureEntry>>(&bytes)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect()
}

// WHY: a write that fails after readers got its entries must take them
// back. Closed here: a failed write whose entries stay readable (the
// window lists a capture the store never held) and one that skips the
// second hook run (the window keeps showing it). Not covered: a failure
// that leaves the store file itself damaged, which the rename rules out.
#[test]
#[serial_test::serial]
#[cfg(unix)]
fn a_failed_write_is_taken_back() {
    use std::os::unix::fs::PermissionsExt;
    let d = isolated_home();
    let (a, img) = png(d.path(), "a.png");
    let (b, _) = png(d.path(), "b.png");
    add(&a, &img).unwrap();
    let data = store_path().unwrap().parent().unwrap().to_path_buf();
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut failed = None;
    let runs = hooked(|| failed = Some(add(&b, &img)));
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(failed.unwrap().is_err(), "a write into a read-only folder");
    assert_eq!(runs, 2, "the hook runs at the write and at its failure");
    let read: Vec<PathBuf> = read_store().into_iter().map(|e| e.path).collect();
    assert_eq!(read, std::slice::from_ref(&a));
    assert_eq!(list().into_iter().map(|e| e.path).collect::<Vec<_>>(), [a]);
}

// WHY: list() sweeps for vanished files without the store lock, and
// prunes under it, so a capture can land between the two. Closed here: a
// prune that drops every path the sweep found gone, a capture saved
// again under one of them since included (the new capture leaves the
// library). Not covered: the interleaving itself, which this test sets
// up by calling the prune with a stale sweep.
#[test]
#[serial_test::serial]
fn a_prune_keeps_a_capture_saved_again_since_the_sweep() {
    let d = isolated_home();
    let (back, img) = png(d.path(), "back.png");
    let (kept, _) = png(d.path(), "kept.png");
    let gone = d.path().join("gone.png");
    add(&back, &img).unwrap();
    add(&kept, &img).unwrap();
    let write = store_lock().lock();
    let mut entries = read_store();
    entries.insert(0, listed(d.path(), "gone.png", 1));
    write_store(entries).unwrap();
    drop(write);
    let dead = std::collections::HashSet::from([back.as_path(), gone.as_path()]);
    let paths = |e: Vec<CaptureEntry>| e.into_iter().map(|e| e.path).collect::<Vec<_>>();
    assert_eq!(paths(prune(&dead)), [kept.clone(), back.clone()]);
    assert_eq!(paths(read_store()), [kept, back]);
}

// WHY: the capture path shows its toast without waiting on the library
// add, and the toast offers Markup, whose save updates the same entry.
// Closed here: a detached add that takes the store after a change made
// once it returned (the save finds no entry, and the add then writes
// over the save's thumbnail and revision). Not covered: a change made
// in another process.
#[test]
#[serial_test::serial]
fn a_detached_add_lands_before_any_later_change() {
    let d = isolated_home();
    let red = image::RgbaImage::from_pixel(300, 300, image::Rgba(RED));
    let green = image::RgbaImage::from_pixel(300, 300, image::Rgba(GREEN));
    for round in 0..20 {
        let shot = d.path().join(format!("{round}.png"));
        red.save(&shot).unwrap();
        add_detached(shot.clone(), std::sync::Arc::new(red.clone()), |e| {
            panic!("the detached add failed: {e}")
        })
        .unwrap();
        let saved = update(&shot, &green).unwrap();
        assert!(
            saved.edited_ms > 0,
            "round {round}: the save found no entry"
        );
        let entries = read_store();
        assert_eq!(entries.len(), round + 1, "round {round}");
        assert_eq!(entries[0].path, shot, "round {round}");
        assert_eq!(entries[0].revision(), saved.revision(), "round {round}");
        assert_eq!(
            thumbnail(&entries[0]).unwrap().get_pixel(0, 0).0,
            GREEN,
            "round {round}: the saved thumbnail"
        );
    }
}

// WHY: the library groups captures by the day they were taken, and an
// editor save over a capture re-added it: the capture moved to the top
// under Today with the save's time as its caption. Closed here: a save
// that moves the entry or changes `created_ms`, a save whose revision
// does not change (the open window keeps the old thumbnail), a save
// after the clock went back that repeats a revision, a save that skips
// the write hook, a thumbnail rebuild from a snapshot older than the
// save that overwrites the save's thumbnail, a save over a file the
// library does not list that is dropped, and a store from before
// `edited_ms` that no longer reads. Not covered: the editor's call,
// which tests/history.rs exercises through the store an editor save
// leaves.
#[test]
#[serial_test::serial]
fn an_editor_save_keeps_the_capture_in_its_place() {
    let d = isolated_home();
    let a = d.path().join("a.png");
    let b = d.path().join("b.png");
    let added = add(&a, &solid(&a, 880, 495, GREEN)).unwrap();
    add(&b, &solid(&b, 880, 495, GREEN)).unwrap();
    let edit = solid(&a, 495, 880, RED);
    let mut saved = None;
    assert_eq!(hooked(|| saved = update(&a, &edit).ok()), 1, "update");
    let saved = saved.unwrap();
    let listed = list();
    assert_eq!(
        listed.iter().map(|e| e.path.clone()).collect::<Vec<_>>(),
        [b.clone(), a.clone()],
        "the store's order after the save"
    );
    let stored = &listed[1];
    assert_eq!(
        (stored.created_ms, stored.width, stored.height),
        (added.created_ms, 495, 880),
        "the saved entry's capture time and size"
    );
    assert_eq!(stored.revision(), saved.revision());
    assert_ne!(
        saved.revision(),
        added.revision(),
        "a save is a new revision"
    );
    assert_eq!(
        image::open(&saved.thumb)
            .unwrap()
            .into_rgba8()
            .get_pixel(0, 0)
            .0,
        RED,
        "the saved thumbnail"
    );
    // A clock set back an hour after the save: the next save is still a
    // new revision.
    let ahead = saved.edited_ms + 3_600_000;
    let mut store = read_store();
    store.iter_mut().find(|e| e.path == a).unwrap().edited_ms = ahead;
    write_store(store).unwrap();
    let again = update(&a, &edit).unwrap();
    assert!(
        again.edited_ms > ahead,
        "a save after the clock went back is a new revision"
    );
    // The window decoding from its snapshot of before the save.
    std::fs::remove_file(&again.thumb).unwrap();
    solid(&a, 880, 495, GREEN);
    thumbnail(&added).unwrap();
    assert!(
        !again.thumb.exists(),
        "a rebuild from before the save wrote over the save's thumbnail"
    );
}

#[test]
#[serial_test::serial]
fn an_editor_save_over_an_unlisted_file_adds_it() {
    let d = isolated_home();
    let a = d.path().join("a.png");
    let saved = update(&a, &solid(&a, 8, 6, GREEN)).unwrap();
    let listed = list();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (&listed[0].path, listed[0].revision()),
        (&a, saved.revision())
    );
}

#[test]
#[serial_test::serial]
fn a_store_written_before_edited_ms_reads() {
    let d = isolated_home();
    let a = d.path().join("a.png");
    solid(&a, 8, 6, GREEN);
    let old = serde_json::json!([{
        "path": a,
        "thumb": d.path().join("t.png"),
        "width": 8,
        "height": 6,
        "created_ms": 7,
    }]);
    std::fs::write(store_path().unwrap(), old.to_string()).unwrap();
    let listed = list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].revision(), (7, 0));
}
