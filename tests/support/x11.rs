//! Window queries, XTest input, and a stand-in compositing manager on a
//! test's X server.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    ImageFormat, MapState, PropMode, Rectangle, WindowClass,
};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::wrapper::ConnectionExt as _;

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

/// The width and height of `window`'s `_NET_WM_ICON`, when it sets one
/// whose length holds them and that many pixels.
pub fn icon_size(conn: &impl Connection, window: u32) -> Option<(u32, u32)> {
    let atom = conn
        .intern_atom(false, b"_NET_WM_ICON")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    let reply = conn
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, u32::MAX / 4)
        .unwrap()
        .reply()
        .ok()?;
    let data: Vec<u32> = reply.value32()?.collect();
    let (w, h) = (*data.first()?, *data.get(1)?);
    (data.len() == 2 + (w * h) as usize).then_some((w, h))
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

/// The window's client area in root pixels: x, y, width, height.
pub fn rect(conn: &impl Connection, root: u32, window: u32) -> (i16, i16, u16, u16) {
    let at = conn
        .translate_coordinates(window, root, 0, 0)
        .unwrap()
        .reply()
        .unwrap();
    let size = conn.get_geometry(window).unwrap().reply().unwrap();
    (at.dst_x, at.dst_y, size.width, size.height)
}

/// The colour the screen shows at `spot`, 0xRRGGBB.
pub fn pixel(conn: &impl Connection, root: u32, spot: (i16, i16)) -> u32 {
    let image = conn
        .get_image(ImageFormat::Z_PIXMAP, root, spot.0, spot.1, 1, 1, !0)
        .unwrap()
        .reply()
        .unwrap();
    // Xvfb stores depth 24 in 32 bits a pixel, blue first.
    u32::from_le_bytes(image.data[..4].try_into().unwrap()) & 0xff_ffff
}

/// Paint the root `rgb`, behind every window, as a desktop does: a root
/// pixmap of that colour, named in `_XROOTPMAP_ID`. A compositing
/// manager such as picom paints the root from that pixmap, and paints
/// black where a root has none. The pixmap lives as long as `conn`.
pub fn paint_root(conn: &impl Connection, root: u32, rgb: u32) {
    let geometry = conn.get_geometry(root).unwrap().reply().unwrap();
    let (width, height) = (geometry.width, geometry.height);
    let pixmap = conn.generate_id().unwrap();
    conn.create_pixmap(geometry.depth, pixmap, root, width, height)
        .unwrap();
    let gc = conn.generate_id().unwrap();
    conn.create_gc(gc, pixmap, &CreateGCAux::new().foreground(rgb))
        .unwrap();
    let all = Rectangle {
        x: 0,
        y: 0,
        width,
        height,
    };
    conn.poly_fill_rectangle(pixmap, gc, &[all]).unwrap();
    conn.free_gc(gc).unwrap();
    let name = conn
        .intern_atom(false, b"_XROOTPMAP_ID")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    conn.change_property32(PropMode::REPLACE, root, name, AtomEnum::PIXMAP, &[pixmap])
        .unwrap();
    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().background_pixmap(pixmap),
    )
    .unwrap();
    conn.clear_area(false, root, 0, 0, 0, 0).unwrap();
    conn.flush().unwrap();
}

/// The left, top, right, and bottom pixels, inclusive, of color `[r, g,
/// b]` in a client capture (`client`), or `None` when it shows none.
pub fn extent(
    (w, _, px): &(usize, usize, Vec<[u8; 3]>),
    [r, g, b]: [u8; 3],
) -> Option<(usize, usize, usize, usize)> {
    let mut found = None;
    for (i, _) in px.iter().enumerate().filter(|&(_, &p)| p == [b, g, r]) {
        let (x, y) = (i % w, i / w);
        let (left, top, right, bottom) = found.get_or_insert((x, y, x, y));
        *left = (*left).min(x);
        *top = (*top).min(y);
        *right = (*right).max(x);
        *bottom = (*bottom).max(y);
    }
    found
}

/// The pixel span of the control that ends nearest left of `before` on
/// one row of pixels: from the first pixel unlike `ground` to the left
/// edge of the run before 8 pixels of `ground`, its label ink included.
/// `None` when the row is `ground` up to `before`.
pub fn control_left_of(row: &[[u8; 3]], ground: [u8; 3], before: usize) -> Option<(usize, usize)> {
    let right = (0..before).rev().find(|&x| row[x] != ground)?;
    let (mut left, mut gap) = (right, 0);
    for x in (0..right).rev() {
        if row[x] == ground {
            gap += 1;
            if gap == 8 {
                break;
            }
        } else {
            gap = 0;
            left = x;
        }
    }
    Some((left, right))
}

/// The pixel span of the text field `width` px wide that ends nearest
/// left of `before` on one row of pixels: the last `width` px of the
/// `control_left_of` span, which also holds a note drawn left of the
/// field. `None` when that span is more than 4 px narrower than `width`.
pub fn field_left_of(
    row: &[[u8; 3]],
    ground: [u8; 3],
    before: usize,
    width: usize,
) -> Option<(usize, usize)> {
    let (left, right) = control_left_of(row, ground, before)?;
    (right + 4 >= left + width).then_some(((right + 1).saturating_sub(width).max(left), right))
}

/// Move the pointer to `at`, through XTest.
pub fn move_to(conn: &impl Connection, root: u32, at: (i16, i16)) {
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
}

/// Move the pointer to `at` and click the first button, through XTest.
pub fn click(conn: &impl Connection, root: u32, at: (i16, i16)) {
    click_with(conn, root, at, 1);
}

/// Move the pointer to `at` and click `button` (3 is the secondary
/// button), through XTest.
pub fn click_with(conn: &impl Connection, root: u32, at: (i16, i16), button: u8) {
    move_to(conn, root, at);
    for kind in [xproto::BUTTON_PRESS_EVENT, xproto::BUTTON_RELEASE_EVENT] {
        conn.xtest_fake_input(kind, button, x11rb::CURRENT_TIME, root, 0, 0, 0)
            .unwrap();
    }
    conn.flush().unwrap();
}

/// Press the first button at `from`, move the pointer to `to` in steps
/// of `step` pixels with a pause after each, and release, through
/// XTest. The pause lets the client read each motion on its own.
pub fn drag(conn: &impl Connection, root: u32, from: (i16, i16), to: (i16, i16), step: i16) {
    press_and_move(conn, root, from, to, step);
    release(conn, root);
}

/// `drag` with the first button still held at `to`.
pub fn press_and_move(
    conn: &impl Connection,
    root: u32,
    from: (i16, i16),
    to: (i16, i16),
    step: i16,
) {
    move_to(conn, root, from);
    std::thread::sleep(std::time::Duration::from_millis(8));
    conn.xtest_fake_input(
        xproto::BUTTON_PRESS_EVENT,
        1,
        x11rb::CURRENT_TIME,
        root,
        0,
        0,
        0,
    )
    .unwrap();
    glide(conn, root, from, to, step);
}

/// Move the pointer from `from` to `to` in steps of `step` pixels with
/// a pause after each, through XTest, buttons as they are.
pub fn glide(conn: &impl Connection, root: u32, from: (i16, i16), to: (i16, i16), step: i16) {
    let (dx, dy) = (i32::from(to.0 - from.0), i32::from(to.1 - from.1));
    let steps = (dx.abs().max(dy.abs()) / i32::from(step.max(1))).max(1);
    for i in 1..=steps {
        move_to(
            conn,
            root,
            (
                from.0 + (dx * i / steps) as i16,
                from.1 + (dy * i / steps) as i16,
            ),
        );
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}

/// Release the first button, through XTest.
pub fn release(conn: &impl Connection, root: u32) {
    conn.xtest_fake_input(
        xproto::BUTTON_RELEASE_EVENT,
        1,
        x11rb::CURRENT_TIME,
        root,
        0,
        0,
        0,
    )
    .unwrap();
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
    let keycodes: Vec<u8> = keysyms.iter().map(|&k| keycode(conn, k)).collect();
    for &keycode in &keycodes {
        key(conn, root, keycode, xproto::KEY_PRESS_EVENT);
    }
    for &keycode in keycodes.iter().rev() {
        key(conn, root, keycode, xproto::KEY_RELEASE_EVENT);
    }
    conn.flush().unwrap();
}

/// Press the first key that produces `keysym` and leave it held,
/// through XTest. `let_go` releases it.
pub fn hold(conn: &impl Connection, root: u32, keysym: u32) {
    key(conn, root, keycode(conn, keysym), xproto::KEY_PRESS_EVENT);
    conn.flush().unwrap();
}

/// Release the first key that produces `keysym`, through XTest.
pub fn let_go(conn: &impl Connection, root: u32, keysym: u32) {
    key(conn, root, keycode(conn, keysym), xproto::KEY_RELEASE_EVENT);
    conn.flush().unwrap();
}

/// The first keycode that produces `keysym`.
fn keycode(conn: &impl Connection, keysym: u32) -> u8 {
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
    low + (at / usize::from(map.keysyms_per_keycode)) as u8
}

/// One XTest key event of `kind` for `keycode`.
fn key(conn: &impl Connection, root: u32, keycode: u8, kind: u8) {
    conn.xtest_fake_input(kind, keycode, x11rb::CURRENT_TIME, root, 0, 0, 0)
        .unwrap();
}

/// Ask `window` to close as a window manager does when its close button
/// is pressed: a `WM_PROTOCOLS` client message holding
/// `WM_DELETE_WINDOW`.
pub fn wm_close(conn: &impl Connection, window: u32) {
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let event = xproto::ClientMessageEvent::new(
        32,
        window,
        atom(b"WM_PROTOCOLS"),
        [atom(b"WM_DELETE_WINDOW"), x11rb::CURRENT_TIME, 0, 0, 0],
    );
    conn.send_event(false, window, xproto::EventMask::NO_EVENT, event)
        .unwrap();
    conn.flush().unwrap();
}

/// The CLIPBOARD selection as UTF-8 text, read through a scratch window,
/// or None when no client owns the selection, the owner offers no text,
/// or it does not answer within 2 s.
pub fn clipboard_text(conn: &impl Connection, root: u32) -> Option<String> {
    String::from_utf8(clipboard(conn, root, b"UTF8_STRING")?).ok()
}

/// The CLIPBOARD selection converted to `target`, read through a scratch
/// window in one property, or None when no client owns the selection,
/// the owner offers no `target`, or it does not answer within 2 s.
pub fn clipboard(conn: &impl Connection, root: u32, target: &[u8]) -> Option<Vec<u8>> {
    let atom = |name: &[u8]| conn.intern_atom(false, name).unwrap().reply().unwrap().atom;
    let (clipboard, target, slot) = (atom(b"CLIPBOARD"), atom(target), atom(b"IRIS_TEST"));
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
    conn.convert_selection(window, clipboard, target, slot, x11rb::CURRENT_TIME)
        .unwrap();
    conn.flush().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut value = None;
    while std::time::Instant::now() < deadline {
        match conn.poll_for_event().unwrap() {
            Some(x11rb::protocol::Event::SelectionNotify(e)) if e.requestor == window => {
                if e.property != x11rb::NONE {
                    let reply = conn
                        .get_property(true, window, slot, AtomEnum::ANY, 0, u32::MAX / 4)
                        .unwrap()
                        .reply()
                        .unwrap();
                    value = Some(reply.value);
                }
                break;
            }
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    conn.destroy_window(window).unwrap();
    conn.flush().unwrap();
    value
}

/// The window that owns the CLIPBOARD selection, or `x11rb::NONE`.
pub fn clipboard_owner(conn: &impl Connection) -> u32 {
    let clipboard = conn
        .intern_atom(false, b"CLIPBOARD")
        .unwrap()
        .reply()
        .unwrap()
        .atom;
    conn.get_selection_owner(clipboard)
        .unwrap()
        .reply()
        .unwrap()
        .owner
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
