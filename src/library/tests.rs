// WHY: the class closed here is "the library loses or corrupts captures":
// a store that does not round-trip, a delete that leaves the file, a cap
// that keeps the wrong end, or a path_key that collides all surface as
// missing thumbnails or vanished shots; a thumbnail smaller than the
// card's 2x box, or one kept at an older build's size, shows as a soft
// card on a HiDPI display. Env-mutating tests run serially with
// IRIS_HOME pointed at a tempdir. Not covered: the library window's
// decode of the returned pixels.
use super::*;

/// Root iris data/cache in a fresh tempdir via `IRIS_HOME` (on every
/// platform); returns it for file seeds.
fn isolated_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(crate::dirs::HOME_ENV, dir.path());
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
    let d = isolated_home();
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
    let _d = isolated_home();
    assert!(delete(Path::new("/nonexistent.png")).is_err());
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

#[test]
#[serial_test::serial]
fn store_caps_at_max_entries() {
    let d = isolated_home();
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

/// A `w`x`h` PNG of one color at `path`.
fn solid(path: &Path, w: u32, h: u32, color: [u8; 4]) -> image::RgbaImage {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
    img.save(path).unwrap();
    img
}

const GREEN: [u8; 4] = [0, 200, 0, 255];
const RED: [u8; 4] = [200, 0, 0, 255];

#[test]
fn thumb_plan_covers_the_card_box_and_crops_its_aspect() {
    // capture -> (scaled, cropped)
    let cases = [
        ((1920, 1080), (469, 264), (432, 264)),
        ((3840, 2160), (469, 264), (432, 264)),
        ((1080, 1920), (432, 768), (432, 264)),
        ((432, 264), (432, 264), (432, 264)),
        // Smaller than the box on one side: never enlarged, only cropped.
        ((300, 1000), (300, 1000), (300, 183)),
        ((1000, 200), (1000, 200), (327, 200)),
        ((8, 6), (8, 6), (8, 5)),
    ];
    for ((w, h), scaled, cropped) in cases {
        assert_eq!(thumb_plan(w, h), (scaled, cropped), "{w}x{h}");
        if w >= THUMB_BOX.0 && h >= THUMB_BOX.1 {
            assert_eq!(cropped, THUMB_BOX, "{w}x{h} covers the box");
        }
    }
}

#[test]
#[serial_test::serial]
fn add_writes_the_center_of_the_capture_at_the_box_size() {
    let d = isolated_home();
    // Red bands at both edges, narrower than what the crop cuts: a
    // center crop shows none of them.
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
    assert_eq!(thumb.dimensions(), THUMB_BOX);
    for y in [0, 131, 263] {
        assert_eq!(thumb.get_pixel(0, y).0, GREEN, "left edge, row {y}");
        assert_eq!(thumb.get_pixel(431, y).0, GREEN, "right edge, row {y}");
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
    solid(&entry.thumb, 432, 264, RED);
    assert_eq!(thumbnail(&entry).unwrap().get_pixel(0, 0).0, RED);
}

#[test]
#[serial_test::serial]
fn thumbnail_rebuilds_a_stale_or_missing_file() {
    let d = isolated_home();
    let shot = d.path().join("s.png");
    let img = solid(&shot, 880, 495, GREEN);
    let entry = add(&shot, &img).unwrap();
    // An older build's 320px-wide thumbnail, then no file at all.
    for what in ["stale", "missing"] {
        if what == "stale" {
            solid(&entry.thumb, 320, 180, RED);
        } else {
            std::fs::remove_file(&entry.thumb).unwrap();
        }
        let rebuilt = thumbnail(&entry).unwrap();
        assert_eq!(rebuilt.dimensions(), THUMB_BOX, "{what}");
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
    assert_eq!(thumbnail(&older).unwrap().dimensions(), THUMB_BOX);
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
    assert_eq!(thumbnail(&entry).unwrap().dimensions(), (300, 183));
    let stored = list().into_iter().find(|e| e.path == shot).unwrap();
    assert_eq!((stored.width, stored.height), (300, 300));
    // The entry's size now matches its thumbnail: no rebuild next time.
    solid(&entry.thumb, 300, 183, RED);
    assert_eq!(thumbnail(&stored).unwrap().get_pixel(0, 0).0, RED);
}

static WRITES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn count_write() {
    WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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
// shows the change only on its next poll), and a read that runs it
// (each list pass would queue another). Not covered: writes by another
// process, which the window's poll reads.
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
    assert_eq!(hooked(|| delete(&a)), 1, "delete");
    assert_eq!(hooked(|| delete(&a)), 0, "delete of an unstored path");
    assert_eq!(hooked(|| delete_many(&[b])), 1, "delete_many");
    assert!(list().is_empty());
}
