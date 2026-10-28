//! Window queries, XTest input, and a stand-in compositing manager on a
//! test's X server.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ConnectionExt as _, CreateWindowAux, ImageFormat, MapState, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;

/// The top-level iris windows titled `title`, mapped or not: WM_CLASS
/// instance and class both `APP_ID`, and WM_NAME `title`.
pub fn all_windows_of(conn: &impl Connection, root: u32, title: &str) -> Vec<u32> {
    let id = iris_lib::APP_ID.as_bytes();
    // A window destroyed since the tree was read answers with an error,
    // and counts as gone.
    let property = |window, atom: AtomEnum| {
        conn.get_property(false, window, atom, AtomEnum::STRING, 0, 64)
            .unwrap()
            .reply()
            .map(|p| p.value)
    };
    let children = conn.query_tree(root).unwrap().reply().unwrap().children;
    children
        .into_iter()
        .filter(|&window| {
            property(window, AtomEnum::WM_CLASS).is_ok_and(|class| {
                let mut parts = class.split(|&b| b == 0);
                parts.next() == Some(id) && parts.next() == Some(id)
            }) && property(window, AtomEnum::WM_NAME).is_ok_and(|name| name == title.as_bytes())
        })
        .collect()
}

/// The mapped top-level iris windows titled `title` (`all_windows_of`).
pub fn windows_of(conn: &impl Connection, root: u32, title: &str) -> Vec<u32> {
    all_windows_of(conn, root, title)
        .into_iter()
        .filter(|&window| {
            conn.get_window_attributes(window)
                .unwrap()
                .reply()
                .is_ok_and(|a| a.map_state == MapState::VIEWABLE)
        })
        .collect()
}

/// The window with the input focus.
pub fn focus(conn: &impl Connection) -> u32 {
    conn.get_input_focus().unwrap().reply().unwrap().focus
}

/// The client area of `window` as it shows on the screen: its width,
/// its height, and its rows of [b, g, r] pixels.
pub fn client(conn: &impl Connection, root: u32, window: u32) -> (usize, usize, Vec<[u8; 3]>) {
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            root,
            at.dst_x,
            at.dst_y,
            size.width,
            size.height,
            !0,
        )
        .unwrap()
        .reply()
        .unwrap();
    let (w, h) = (usize::from(size.width), usize::from(size.height));
    assert_eq!(image.data.len(), w * h * 4, "depth {}", image.depth);
    let px = image.data.chunks(4).map(|p| [p[0], p[1], p[2]]).collect();
    (w, h, px)
}

/// Move the pointer to `at` and click the first button, through XTest.
pub fn click(conn: &impl Connection, root: u32, at: (i16, i16)) {
    conn.xtest_fake_input(
        xproto::MOTION_NOTIFY_EVENT,
        0,
        x11rb::CURRENT_TIME,
        root,
        at.0,
        at.1,
        0,
    )
    .unwrap();
    for kind in [xproto::BUTTON_PRESS_EVENT, xproto::BUTTON_RELEASE_EVENT] {
        conn.xtest_fake_input(kind, 1, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
    }
    conn.flush().unwrap();
}

/// Press the first button at `from`, move the pointer to `to` in steps
/// of `step` pixels with a pause after each, and release, through
/// XTest. The pause lets the client read each motion on its own.
pub fn drag(conn: &impl Connection, root: u32, from: (i16, i16), to: (i16, i16), step: i16) {
    let motion = |at: (i16, i16)| {
        conn.xtest_fake_input(
            xproto::MOTION_NOTIFY_EVENT,
            0,
            x11rb::CURRENT_TIME,
            root,
            at.0,
            at.1,
            0,
        )
        .unwrap();
        conn.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(8));
    };
    let button = |kind| {
        conn.xtest_fake_input(kind, 1, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
    };
    motion(from);
    button(xproto::BUTTON_PRESS_EVENT);
    let (dx, dy) = (i32::from(to.0 - from.0), i32::from(to.1 - from.1));
    let steps = (dx.abs().max(dy.abs()) / i32::from(step.max(1))).max(1);
    for i in 1..=steps {
        motion((
            from.0 + (dx * i / steps) as i16,
            from.1 + (dy * i / steps) as i16,
        ));
    }
    button(xproto::BUTTON_RELEASE_EVENT);
    conn.flush().unwrap();
}

/// Press and release the first key that produces `keysym`, through
/// XTest, with no modifier held.
pub fn tap(conn: &impl Connection, root: u32, keysym: u32) {
    chord(conn, root, &[keysym]);
}

/// Press the first key that produces each of `keysyms` in order, then
/// release them in reverse order, through XTest: `[CONTROL_L, PRINT]`
/// is Ctrl+Print.
pub fn chord(conn: &impl Connection, root: u32, keysyms: &[u32]) {
    let (low, high) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn
        .get_keyboard_mapping(low, high - low + 1)
        .unwrap()
        .reply()
        .unwrap();
    let keycodes: Vec<u8> = keysyms
        .iter()
        .map(|&keysym| {
            let at = map
                .keysyms
                .iter()
                .position(|&k| k == keysym)
                .unwrap_or_else(|| panic!("no key produces keysym {keysym:#x}"));
            low + (at / usize::from(map.keysyms_per_keycode)) as u8
        })
        .collect();
    let fake = |kind, keycode| {
        conn.xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
    };
    for &keycode in &keycodes {
        fake(xproto::KEY_PRESS_EVENT, keycode);
    }
    for &keycode in keycodes.iter().rev() {
        fake(xproto::KEY_RELEASE_EVENT, keycode);
    }
    conn.flush().unwrap();
}

/// A compositing manager: a window that owns the screen's
/// `_NET_WM_CM_S<n>` selection, and that selection.
pub struct Compositor {
    window: u32,
    selection: u32,
}

impl Compositor {
    pub fn start(conn: &impl Connection, root: u32, selection: u32) -> Compositor {
        let window = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .unwrap();
        conn.set_selection_owner(window, selection, x11rb::CURRENT_TIME)
            .unwrap();
        let owner = conn
            .get_selection_owner(selection)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(owner.owner, window, "the compositor owns its selection");
        Compositor { window, selection }
    }

    /// Exit: the selection of a destroyed window has no owner.
    pub fn exit(self, conn: &impl Connection) {
        conn.destroy_window(self.window).unwrap();
        let owner = conn
            .get_selection_owner(self.selection)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(
            owner.owner,
            x11rb::NONE,
            "the compositor released its selection"
        );
    }
}
