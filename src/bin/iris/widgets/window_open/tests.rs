// WHY: every iris window opens through `open_window` (clippy.toml
// disallows the other openers), so the options it hands GPUI are the one
// place two per-app settings hold for every window. Closed here: a window
// that opens with GPUI's default 30-per-second cap on its animations while
// it has no focus, which halved the frame rate of every pop-up animation
// (a pop-up never takes focus), and a window whose app id is not the
// desktop entry's. Each case sets both fields to something else first, so
// an override that a caller's options replace also fails. The other fields
// are the caller's.
// Not covered: what GPUI does with the options, and whether a window reads
// as focused on a given platform.

use std::prelude::v1::test;
use std::time::Duration;

use gpui::{
    point, px, size, ActivationPolicy, Bounds, WindowBounds, WindowId, WindowKind, WindowOptions,
};

use super::{iris_options, AppWindows};

#[test]
fn every_window_animates_unthrottled_under_the_iris_app_id() {
    let intervals = [
        WindowOptions::default().inactive_frame_interval,
        Some(Duration::from_millis(1)),
        None,
    ];
    let app_ids = [None, Some("dev.other.app".to_owned())];
    let kinds = [
        WindowKind::Normal,
        WindowKind::PopUp,
        WindowKind::Floating,
        WindowKind::Dialog,
    ];
    for kind in kinds {
        for &interval in &intervals {
            for app_id in &app_ids {
                let options = iris_options(WindowOptions {
                    kind: kind.clone(),
                    inactive_frame_interval: interval,
                    app_id: app_id.clone(),
                    ..Default::default()
                });
                let case = format!("{kind:?}, interval {interval:?}, app id {app_id:?}");
                assert_eq!(options.inactive_frame_interval, None, "{case}");
                assert_eq!(options.app_id.as_deref(), Some(iris_lib::APP_ID), "{case}");
                assert_eq!(options.kind, kind, "{case}");
            }
        }
    }
}

#[test]
fn the_callers_other_options_pass_through() {
    let bounds = Bounds {
        origin: point(px(-1920.0), px(40.0)),
        size: size(px(360.0), px(120.0)),
    };
    for (focus, show) in [(false, true), (true, false)] {
        let options = iris_options(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            focus,
            show,
            is_movable: !focus,
            is_resizable: focus,
            ..Default::default()
        });
        assert_eq!(
            options.window_bounds,
            Some(WindowBounds::Windowed(bounds)),
            "focus {focus}"
        );
        assert_eq!(
            (
                options.focus,
                options.show,
                options.is_movable,
                options.is_resizable
            ),
            (focus, show, !focus, focus)
        );
    }
}

/// WHY: iris has a Dock icon and a menu bar on macOS while an app window
/// is open and neither once the last one closes. Closed here: a second
/// app window, a reopened one, or the close of a window that is not an
/// app window (the overlay, a pin) that changes the policy, and a last
/// close that leaves iris in the Dock. Not covered: what AppKit does with
/// the policy, which only runs on macOS.
#[test]
fn only_the_first_open_and_the_last_close_change_the_dock() {
    let (a, b, other) = (WindowId::from(1), WindowId::from(2), WindowId::from(3));
    let mut open = AppWindows::default();
    let steps: [(&str, bool, WindowId, Option<ActivationPolicy>); 9] = [
        ("open a", true, a, Some(ActivationPolicy::Regular)),
        ("open b", true, b, None),
        ("open a again", true, a, None),
        ("close a pop-up", false, other, None),
        ("close a", false, a, None),
        ("close a again", false, a, None),
        ("close b", false, b, Some(ActivationPolicy::Accessory)),
        ("close b again", false, b, None),
        ("open b after", true, b, Some(ActivationPolicy::Regular)),
    ];
    for (step, opens, id, want) in steps {
        let got = if opens {
            open.opened(id)
        } else {
            open.closed(id)
        };
        assert_eq!(got, want, "{step}");
    }
}

/// WHY: an X11 app window shows the iris icon in a taskbar with no
/// desktop entry for its class only when the embedded PNG decodes; a
/// decode failure logs and leaves the generic icon. Closed here: an
/// embedded file that does not decode or has another size. Not covered:
/// which icon a window manager draws.
#[cfg(target_os = "linux")]
#[test]
fn the_window_icon_decodes_at_its_size() {
    let icon = super::app_icon().expect("the embedded icon decodes");
    assert_eq!(icon.dimensions(), (128, 128));
    assert!(
        icon.pixels().any(|p| p.0[3] == 255),
        "the icon has opaque pixels"
    );
}
