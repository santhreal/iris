// WHY: the class closed here is "a window's controls in another
// platform's register or place". Every window drew the macOS traffic
// lights at its top-left corner: on Windows the caption buttons belong
// at the trailing edge, and on Linux the desktop's button layout sets
// the side and the buttons, with GNOME's default a lone close button at
// the trailing edge. Each case is a register, a desktop layout, and a
// window's resizability, with the buttons each side draws. Not covered:
// how the buttons look, which the screenshots show; AppKit's own
// buttons on macOS, which `open_window` places.

use std::prelude::v1::test;

use gpui::WindowButton::{self, Close, Maximize, Minimize};
use gpui::WindowButtonLayout;

use super::{sides, Register};

/// A register, the desktop's layout, whether the window resizes, and
/// the buttons expected at the leading and trailing edges.
type Case = (
    Register,
    Option<WindowButtonLayout>,
    bool,
    &'static [WindowButton],
    &'static [WindowButton],
);

fn layout(left: &[WindowButton], right: &[WindowButton]) -> WindowButtonLayout {
    let side = |buttons: &[WindowButton]| {
        let mut out = [None; 3];
        for (slot, &b) in out.iter_mut().zip(buttons) {
            *slot = Some(b);
        }
        out
    };
    WindowButtonLayout {
        left: side(left),
        right: side(right),
    }
}

/// Every register, so that a new one fails to compile here until it
/// has cases.
fn registers() -> [Register; 3] {
    let all = [Register::Mac, Register::Windows, Register::Linux];
    for r in all {
        match r {
            Register::Mac | Register::Windows | Register::Linux => {}
        }
    }
    all
}

#[test]
fn each_platform_places_its_window_controls_where_its_own_windows_do() {
    let mac_left = layout(&[Close, Minimize, Maximize], &[]);
    let gnome = layout(&[], &[Close]);
    let split = layout(&[Close], &[Minimize, Maximize]);
    #[rustfmt::skip]
    let cases: &[Case] = &[
        // AppKit draws the macOS buttons; the toolbar draws none.
        (Register::Mac, None, true, &[], &[]),
        (Register::Mac, Some(mac_left), false, &[], &[]),
        // Windows: all three at the trailing edge, whatever the layout,
        // and a fixed-size window keeps its dimmed maximize button.
        (Register::Windows, None, true, &[], &[Minimize, Maximize, Close]),
        (Register::Windows, Some(gnome), false, &[], &[Minimize, Maximize, Close]),
        // Linux with no layout: trailing, no maximize on a fixed size.
        (Register::Linux, None, true, &[], &[Minimize, Maximize, Close]),
        (Register::Linux, None, false, &[], &[Minimize, Close]),
        // Linux follows the desktop's layout, side and order.
        (Register::Linux, Some(gnome), true, &[], &[Close]),
        (Register::Linux, Some(mac_left), true, &[Close, Minimize, Maximize], &[]),
        (Register::Linux, Some(mac_left), false, &[Close, Minimize], &[]),
        (Register::Linux, Some(split), true, &[Close], &[Minimize, Maximize]),
        (Register::Linux, Some(split), false, &[Close], &[Minimize]),
    ];
    for register in registers() {
        assert!(
            cases.iter().any(|c| c.0 == register),
            "{register:?} has no case"
        );
    }
    let mut wrong = Vec::new();
    for &(register, desktop, resizable, left, right) in cases {
        let got = sides(register, desktop, resizable);
        let want = layout(left, right);
        if got != want {
            wrong.push(format!(
                "{register:?}, layout {desktop:?}, resizable {resizable}: {got:?}, expected {want:?}"
            ));
        }
    }
    assert!(wrong.is_empty(), "window controls:\n{}", wrong.join("\n"));
}
