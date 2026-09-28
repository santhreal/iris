//! Window queries, XTest input, and a stand-in compositing manager on a
//! test's X server.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ConnectionExt as _, CreateWindowAux, MapState, WindowClass,
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

/// Press and release the first key that produces `keysym`, through
/// XTest, with no modifier held.
pub fn tap(conn: &impl Connection, root: u32, keysym: u32) {
    let (low, high) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn
        .get_keyboard_mapping(low, high - low + 1)
        .unwrap()
        .reply()
        .unwrap();
    let at = map
        .keysyms
        .iter()
        .position(|&k| k == keysym)
        .unwrap_or_else(|| panic!("no key produces keysym {keysym:#x}"));
    let keycode = low + (at / usize::from(map.keysyms_per_keycode)) as u8;
    for kind in [xproto::KEY_PRESS_EVENT, xproto::KEY_RELEASE_EVENT] {
        conn.xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
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
