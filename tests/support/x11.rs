//! Window queries and XTest input on a test's X server.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, MapState};
use x11rb::protocol::xtest::ConnectionExt as _;

/// The mapped top-level iris windows titled `title`: WM_CLASS instance
/// and class both `APP_ID`, and WM_NAME `title`.
pub fn windows_of(conn: &impl Connection, root: u32, title: &str) -> Vec<u32> {
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
            let mapped = conn
                .get_window_attributes(window)
                .unwrap()
                .reply()
                .is_ok_and(|a| a.map_state == MapState::VIEWABLE);
            mapped
                && property(window, AtomEnum::WM_CLASS).is_ok_and(|class| {
                    let mut parts = class.split(|&b| b == 0);
                    parts.next() == Some(id) && parts.next() == Some(id)
                })
                && property(window, AtomEnum::WM_NAME).is_ok_and(|name| name == title.as_bytes())
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
