//! The clipboard: one `arboard::Clipboard` for the life of the process.
//!
//! On X11 the selection is served from the instance that set it, so the
//! process keeps one instead of opening one per copy. On Wayland,
//! arboard sets the selection through the compositor's data-control
//! protocol (`ext-data-control-v1` or `wlr-data-control-unstable-v1`),
//! which needs no focused window, and uses the X server in `DISPLAY`
//! (XWayland) on a compositor without one. The instance opens on the
//! first copy: a daemon that never copies connects to neither.
//!
//! Each call waits on the X server or the compositor, and the first
//! can start an on-demand XWayland: call them off the UI thread.

use std::borrow::Cow;
use std::sync::LazyLock;

use parking_lot::Mutex;

/// None when no clipboard could be opened: a capture still saves.
static CLIPBOARD: LazyLock<Option<Mutex<arboard::Clipboard>>> = LazyLock::new(|| {
    arboard::Clipboard::new()
        .map(Mutex::new)
        .map_err(|e| crate::ilog!("iris: clipboard unavailable: {e}"))
        .ok()
});

/// Run `f` on the process's clipboard; `what` prefixes its error.
fn with(
    what: &str,
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<(), arboard::Error>,
) -> Result<(), String> {
    let clipboard = CLIPBOARD.as_ref().ok_or("clipboard unavailable")?;
    f(&mut clipboard.lock()).map_err(|e| format!("{what}: {e}"))
}

/// Put RGBA pixels on the clipboard. On Linux arboard offers them as
/// `image/png`.
pub fn set_image(img: &image::RgbaImage) -> Result<(), String> {
    with("set clipboard image", |c| {
        c.set_image(arboard::ImageData {
            width: img.width() as usize,
            height: img.height() as usize,
            bytes: Cow::Borrowed(img.as_raw()),
        })
    })
}

pub fn set_text(text: &str) -> Result<(), String> {
    with("set clipboard text", |c| c.set_text(text))
}

/// Put files on the clipboard as a `text/uri-list`, which file managers
/// paste as the files themselves.
#[cfg(target_os = "linux")]
pub fn set_file_list(paths: &[std::path::PathBuf]) -> Result<(), String> {
    with("set clipboard file list", |c| c.set().file_list(paths))
}

// WHY: the class closed here is "a copy that no paste can read": text,
// an image, or a file list set on the process's clipboard must reach a
// separate X client that asks for the target a paste asks for, as the
// same text, the same pixels, and the same files in order. One test,
// because every case owns the same CLIPBOARD selection. Runs only with
// IRIS_X11_TEST_DISPLAY (a private Xvfb), so it never touches a desktop
// session's clipboard; tests/clipboard.rs covers Wayland. Not covered:
// INCR transfers of payloads over the X server's request size.
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::time::{Duration, Instant};

    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, Window, WindowClass,
    };
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    /// A second X client that pastes CLIPBOARD as an application does,
    /// into a property of its own input-only window.
    struct Paster {
        conn: RustConnection,
        win: Window,
    }

    impl Paster {
        fn new() -> Self {
            let (conn, screen) = x11rb::connect(None).unwrap();
            let root = conn.setup().roots[screen].root;
            let win = conn.generate_id().unwrap();
            conn.create_window(
                0,
                win,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap();
            Paster { conn, win }
        }

        fn atom(&self, name: &str) -> Atom {
            let cookie = self.conn.intern_atom(false, name.as_bytes()).unwrap();
            cookie.reply().unwrap().atom
        }

        /// CLIPBOARD converted to `target`, or None when the owner
        /// refuses the target. Fails the test when no answer arrives by
        /// `deadline`, so a hung owner cannot hang the suite.
        fn paste(&self, target: &str, deadline: Instant) -> Option<Vec<u8>> {
            let (clipboard, target_atom, prop) = (
                self.atom("CLIPBOARD"),
                self.atom(target),
                self.atom("IRIS_PASTE"),
            );
            self.conn
                .convert_selection(self.win, clipboard, target_atom, prop, x11rb::CURRENT_TIME)
                .unwrap();
            self.conn.flush().unwrap();
            loop {
                match self.conn.poll_for_event().unwrap() {
                    Some(Event::SelectionNotify(n)) if n.property == x11rb::NONE => return None,
                    Some(Event::SelectionNotify(_)) => {
                        let reply = self
                            .conn
                            .get_property(true, self.win, prop, AtomEnum::ANY, 0, u32::MAX / 4)
                            .unwrap()
                            .reply()
                            .unwrap();
                        return Some(reply.value);
                    }
                    Some(_) => {}
                    None => {
                        assert!(
                            Instant::now() < deadline,
                            "{target}: the owner never answered"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }

        /// Paste `target` until it reads `want`, for at most 5 s.
        fn expect(&self, target: &str, want: &[u8]) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let got = self.paste(target, deadline);
                if got.as_deref() == Some(want) {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{target}: pasted {:?}, want {:?}",
                    got.map(|g| String::from_utf8_lossy(&g).into_owned()),
                    String::from_utf8_lossy(want)
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        /// Paste `target` once the owner offers it, for at most 5 s.
        fn offered(&self, target: &str) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(bytes) = self.paste(target, deadline) {
                    return bytes;
                }
                assert!(Instant::now() < deadline, "{target}: never offered");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn each_kind_pastes_back_on_x11() {
        let Some(display) = std::env::var_os("IRIS_X11_TEST_DISPLAY") else {
            return;
        };
        std::env::set_var("DISPLAY", &display);
        std::env::remove_var("WAYLAND_DISPLAY");
        let paster = Paster::new();

        super::set_text("#a1b2c3").unwrap();
        paster.expect("UTF8_STRING", b"#a1b2c3");

        let img = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([x as u8 * 80, y as u8 * 90, 7, 255])
        });
        super::set_image(&img).unwrap();
        let png = paster.offered("image/png");
        assert_eq!(image::load_from_memory(&png).unwrap().to_rgba8(), img);

        // A space and a second file: the list names each file, in order.
        let dir = tempfile::tempdir().unwrap();
        let files = [dir.path().join("a b.png"), dir.path().join("c.png")];
        for f in &files {
            std::fs::write(f, b"x").unwrap();
        }
        crate::dragcopy::copy_file_paths(&files).unwrap();
        let list = String::from_utf8(paster.offered("text/uri-list")).unwrap();
        let pasted: Vec<_> = list
            .lines()
            .map(|uri| {
                uri.strip_prefix("file://")
                    .map(crate::dragcopy::uri_decode_path)
            })
            .collect();
        let want: Vec<_> = files.iter().map(|f| f.canonicalize().ok()).collect();
        assert_eq!(pasted, want, "uri-list {list:?}");
    }
}
