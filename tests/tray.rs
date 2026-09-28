//! The tray item's answer to a primary click, read over a private
//! session bus.
//!
//! WHY: the class closed here is "a primary click on the tray icon opens
//! nothing on Linux". On a primary click a panel reads the
//! StatusNotifierItem's `ItemIsMenu` property and calls its `Activate`
//! method: Plasma 6.4 and later open the menu when `ItemIsMenu` is true,
//! and GNOME's AppIndicator extension and Plasma before 6.4 open it when
//! `Activate` fails with `UnknownMethod`. The item reported false and
//! answered `Activate` with success, so a primary click opened nothing,
//! where on Windows and macOS it opens the menu. The case starts the
//! daemon on a private Xvfb and a private dbus-daemon and reads both
//! answers from the item.
//!
//! Not covered: the menu a panel draws from the item's dbusmenu (its rows
//! are the table in src/bin/iris/sys/tray.rs, covered by its unit
//! tests); a panel itself; Windows and macOS. The case runs only with
//! `IRIS_X11_TEST_DISPLAY` set and needs Xvfb and dbus-daemon; without
//! them it prints that it did not run.

#![cfg(target_os = "linux")]

// The other test files use the rest of the harness.
#[allow(dead_code)]
#[path = "support/daemon.rs"]
mod daemon;

use daemon::{enabled, session_bus, xvfb, Daemon};
use zbus::zvariant::OwnedValue;

/// The StatusNotifierItem interface, and the prefix of the bus name the
/// item takes: `<ITEM>-<pid>-<n>`.
const ITEM: &str = "org.kde.StatusNotifierItem";
/// The item's object path.
const PATH: &str = "/StatusNotifierItem";

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
    let bus = zbus::block_on(
        zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .build(),
    )
    .unwrap();

    let prefix = format!("{ITEM}-{}-", daemon.pid());
    let item = daemon.until("the tray item to take its bus name", || {
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
    });

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
}
