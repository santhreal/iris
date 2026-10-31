//! The tray item's answer to a primary click, its icon, and its menu
//! during a recording, read over a private session bus.
//!
//! WHY: three classes close here. "A primary click on the tray icon opens
//! nothing on Linux": on a primary click a panel reads the
//! StatusNotifierItem's `ItemIsMenu` property and calls its `Activate`
//! method: Plasma 6.4 and later open the menu when `ItemIsMenu` is true,
//! and GNOME's AppIndicator extension and Plasma before 6.4 open it when
//! `Activate` fails with `UnknownMethod`. The item reported false and
//! answered `Activate` with success, so a primary click opened nothing,
//! where on Windows and macOS it opens the menu. The first case starts
//! the daemon on a private Xvfb and a private dbus-daemon and reads both
//! answers from the item. "The panel draws a jagged or miscolored
//! glyph": the item served one 22 px raster with its bytes in RGBA
//! order; the same case reads `IconPixmap` and holds every raster to
//! ARGB32 white at its own size.
//! "The tray menu does not follow the recording": the menu offered
//! **Record Window** while a recording ran, and that row stopped it.
//! The second case starts a window recording, reads **Stop Recording**
//! in the menu the item serves, clicks it through the menu's `Event`
//! method, and reads the recording saved and **Record Window** back.
//!
//! Not covered: a panel itself; a recording that ends on its own, whose
//! end takes the same path (`daemon::recording::Live`, covered by its
//! unit test); Windows and macOS, which draw from the same table. The
//! cases run only with `IRIS_X11_TEST_DISPLAY` set and need Xvfb and
//! dbus-daemon, and the second needs ffmpeg; without them they print
//! that they did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/recording.rs"]
mod recording;
// The other test files use the rest of the helpers.
#[allow(dead_code)]
#[path = "support/x11.rs"]
mod x11;

use std::collections::HashMap;

use daemon::{enabled, session_bus, xvfb, Daemon};
use recording::Recording;
use zbus::zvariant::{OwnedValue, Value};

/// The StatusNotifierItem interface, and the prefix of the bus name the
/// item takes: `<ITEM>-<pid>-<n>`.
const ITEM: &str = "org.kde.StatusNotifierItem";
/// The item's object path.
const PATH: &str = "/StatusNotifierItem";
/// The menu's interface and object path.
const MENU: &str = "com.canonical.dbusmenu";
const MENU_PATH: &str = "/MenuBar";
/// The menu's item labels with no update offered.
const IDLE: [&str; 5] = [
    "New Screenshot",
    "Record Window",
    "Library",
    "Settings",
    "Quit",
];
const RECORDING: [&str; 5] = [
    "New Screenshot",
    "Stop Recording",
    "Library",
    "Settings",
    "Quit",
];

/// A connection to the private bus at `address`.
fn connect(address: &str) -> zbus::Connection {
    zbus::block_on(zbus::connection::Builder::address(address).unwrap().build()).unwrap()
}

/// The bus name `daemon`'s tray item takes, once it takes it.
fn tray_item(daemon: &Daemon, bus: &zbus::Connection) -> String {
    let prefix = format!("{ITEM}-{}-", daemon.pid());
    daemon.until("the tray item to take its bus name", || {
        let reply = zbus::block_on(bus.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "ListNames",
            &(),
        ))
        .unwrap();
        let names: Vec<String> = reply.body().deserialize().unwrap();
        names.into_iter().find(|name| name.starts_with(&prefix))
    })
}

/// The menu's labelled items in order, with their menu ids.
fn menu(bus: &zbus::Connection, item: &str) -> Vec<(i32, String)> {
    let reply = zbus::block_on(bus.call_method(
        Some(item),
        MENU_PATH,
        Some(MENU),
        "GetGroupProperties",
        &(Vec::<i32>::new(), vec!["label"]),
    ))
    .unwrap();
    let mut items: Vec<(i32, HashMap<String, OwnedValue>)> = reply.body().deserialize().unwrap();
    items.sort_by_key(|(id, _)| *id);
    items
        .into_iter()
        .filter_map(|(id, mut props)| {
            let label = String::try_from(props.remove("label")?).ok()?;
            (!label.is_empty()).then_some((id, label))
        })
        .collect()
}

/// Wait until the menu's labels are `want`, and return the menu.
fn menu_is(
    daemon: &Daemon,
    bus: &zbus::Connection,
    item: &str,
    want: [&str; 5],
) -> Vec<(i32, String)> {
    daemon.until(&format!("the tray menu to read {want:?}"), || {
        let menu = menu(bus, item);
        menu.iter()
            .map(|(_, label)| label.as_str())
            .eq(want)
            .then_some(menu)
    })
}

#[test]
fn a_primary_click_on_the_tray_icon_opens_its_menu() {
    let case = "a_primary_click_on_the_tray_icon_opens_its_menu";
    if !enabled() {
        return;
    }
    let Some((_xvfb, display)) = xvfb(case) else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let Some((_bus, address)) = session_bus(case, dir.path()) else {
        return;
    };
    let daemon = Daemon::start(dir.path(), |cmd| {
        cmd.env("DISPLAY", &display)
            .env_remove("WAYLAND_DISPLAY")
            .env("DBUS_SESSION_BUS_ADDRESS", &address);
    });
    let bus = connect(&address);
    let item = tray_item(&daemon, &bus);

    let reply = zbus::block_on(bus.call_method(
        Some(item.as_str()),
        PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(ITEM, "ItemIsMenu"),
    ))
    .unwrap();
    let is_menu = bool::try_from(reply.body().deserialize::<OwnedValue>().unwrap()).unwrap();
    assert!(
        is_menu,
        "the tray item reports ItemIsMenu false, so Plasma 6.4 and later activate it on a \
         primary click instead of opening its menu"
    );

    let activated = zbus::block_on(bus.call_method(
        Some(item.as_str()),
        PATH,
        Some(ITEM),
        "Activate",
        &(0i32, 0i32),
    ));
    match activated {
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" => {}
        other => panic!(
            "the tray item answered Activate with {other:?}, not UnknownMethod, so GNOME's \
             AppIndicator extension and Plasma before 6.4 open no menu on a primary click"
        ),
    }

    // The panel draws the glyph from IconPixmap: one ARGB32 raster per
    // size, white wherever it is covered, so a panel scales none.
    let reply = zbus::block_on(bus.call_method(
        Some(item.as_str()),
        PATH,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(ITEM, "IconPixmap"),
    ))
    .unwrap();
    let pixmaps = reply.body().deserialize::<OwnedValue>().unwrap();
    let Value::Array(pixmaps) = &*pixmaps else {
        panic!("IconPixmap is {pixmaps:?}, not an array");
    };
    let mut sizes = Vec::new();
    for pixmap in pixmaps.iter() {
        let Value::Structure(pixmap) = pixmap else {
            panic!("an IconPixmap entry is {pixmap:?}, not (iiay)");
        };
        let [Value::I32(w), Value::I32(h), Value::Array(data)] = pixmap.fields() else {
            panic!("an IconPixmap entry is {pixmap:?}, not (iiay)");
        };
        let data: Vec<u8> = data
            .iter()
            .map(|b| match b {
                Value::U8(b) => *b,
                other => panic!("an IconPixmap byte is {other:?}"),
            })
            .collect();
        assert_eq!(data.len(), (w * h * 4) as usize, "{w}x{h}");
        assert!(
            data.chunks(4)
                .all(|p| p[0] == 0 || p[1..] == [255, 255, 255]),
            "the {w}x{h} pixmap is not a white glyph in ARGB order"
        );
        sizes.push((*w, *h));
    }
    assert_eq!(sizes, [(16, 16), (22, 22), (24, 24), (32, 32), (48, 48)]);
}

#[test]
fn the_tray_menu_offers_stop_while_a_recording_runs() {
    let case = "the_tray_menu_offers_stop_while_a_recording_runs";
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let Some((_bus, address)) = session_bus(case, dir.path()) else {
        return;
    };
    let Some(rec) = Recording::start_with(case, |cmd| {
        cmd.env("DBUS_SESSION_BUS_ADDRESS", &address);
    }) else {
        return;
    };
    let bus = connect(&address);
    let item = tray_item(&rec.daemon, &bus);
    let shown = menu_is(&rec.daemon, &bus, &item, RECORDING);
    let stop = shown[1].0;
    zbus::block_on(bus.call_method(
        Some(item.as_str()),
        MENU_PATH,
        Some(MENU),
        "Event",
        &(stop, "clicked", Value::from(0i32), 0u32),
    ))
    .unwrap();
    rec.daemon.until("the recording to be saved", || {
        rec.daemon
            .log()
            .contains("iris: recording saved")
            .then_some(())
    });
    rec.saved();
    menu_is(&rec.daemon, &bus, &item, IDLE);
}
