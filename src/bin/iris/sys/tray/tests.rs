//! The tray menu table and its dispatch.
//!
//! Every platform draws `MENU` and reports a click as a row id, so a
//! wrong id mapping sends the wrong command on all three at once. These
//! tests walk the table itself: a new row is covered without an edit
//! here. They do not click a live tray.

use futures::channel::mpsc;

use super::{
    command, drawn, pick, rows, set_offer, set_recording, Drawn, Row, When, MENU, TX, UPDATE_LABEL,
};
use crate::daemon::{line_wait, MENU_FADE};

fn debug(cmd: Option<crate::daemon::Command>) -> String {
    format!("{cmd:?}")
}

/// The record row's label: Linux records a picked window, Windows and
/// macOS the desktop.
const RECORD: &str = if cfg!(target_os = "linux") {
    "Record Window"
} else {
    "Record Screen"
};

#[test]
fn every_row_id_maps_to_that_rows_command() {
    for (id, row) in rows() {
        let want = match row {
            Row::Item { command, .. } => Some(command.clone()),
            Row::Update => Some(crate::daemon::Command::InstallUpdate),
            Row::Separator => None,
        };
        assert_eq!(debug(command(id)), debug(want), "row id {id}");
    }
}

#[test]
fn ids_start_at_one_because_zero_means_no_item() {
    assert_eq!(rows().next().map(|(id, _)| id), Some(1));
    assert!(command(0).is_none());
    assert!(command(MENU.len() + 1).is_none());
    assert!(command(usize::MAX).is_none());
}

// WHY: the class closed here is "a tray row freezes the tray menu into
// the screen it reads". The click that picks a row arrives while the
// shell's menu is still fading out, and a capture sent at once froze a
// frame holding the menu with the clicked row lit. Every row is walked
// from `MENU`: a row whose command `line_wait` holds for a menu is sent
// no sooner than `MENU_FADE` after the click, and every other row at
// once. Which commands wait is held in the cli tests. Not covered: how
// long a given shell's fade runs.
#[test]
fn a_pick_sends_its_command_once_the_menu_has_faded() {
    use std::time::{Duration, Instant};
    let (tx, mut rx) = mpsc::unbounded();
    assert!(TX.set(tx).is_ok(), "only this test sets the channel");
    for (id, row) in rows() {
        let start = Instant::now();
        pick(id);
        let want = match row {
            Row::Item { command, .. } => Some(command.clone()),
            Row::Update => Some(crate::daemon::Command::InstallUpdate),
            Row::Separator => None,
        };
        let sends = want.is_some();
        let waits = want
            .as_ref()
            .is_some_and(|c| !line_wait(std::slice::from_ref(c), true).is_zero());
        if waits {
            assert!(
                rx.try_recv().is_err(),
                "row id {id} sent its command while the menu still fades"
            );
        }
        let sent = loop {
            match rx.try_recv() {
                Ok(cmd) => break Some(cmd),
                Err(_) if start.elapsed() > MENU_FADE * 5 => break None,
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        };
        let took = start.elapsed();
        assert_eq!(debug(sent), debug(want), "row id {id}");
        if waits {
            assert!(took >= MENU_FADE, "row id {id} sent after {took:?}");
        } else if sends {
            assert!(took < MENU_FADE, "row id {id} waited {took:?}");
        }
    }
    assert!(
        rows().any(|(id, _)| command(id).is_some_and(|c| !line_wait(
            std::slice::from_ref(&c),
            true
        )
        .is_zero())),
        "no row starts reading the screen, so no pick waits"
    );
    pick(0);
    pick(MENU.len() + 1);
    std::thread::sleep(MENU_FADE + Duration::from_millis(100));
    assert!(rx.try_recv().is_err());
}

/// Each label runs the command its usage-chapter row describes, the
/// record row gives way to the stop row while a recording runs, the
/// install row sits with Settings, and Quit sits alone below the
/// separator.
#[test]
fn each_label_runs_its_documented_command() {
    let menu: Vec<String> = MENU
        .iter()
        .map(|row| match row {
            Row::Item {
                label,
                command,
                when,
            } => format!("{label}: {command:?} ({when:?})"),
            Row::Update => format!("{UPDATE_LABEL} <version>: InstallUpdate"),
            Row::Separator => "---".into(),
        })
        .collect();
    assert_eq!(
        menu,
        [
            "New Screenshot: Capture (Always)",
            format!("{RECORD}: RecordToggle (Idle)").as_str(),
            "Stop Recording: RecordStop (Recording)",
            "Library: Library (Always)",
            "Settings: Settings (Always)",
            "Install iris <version>: InstallUpdate",
            "---",
            "Quit: Quit (Always)",
        ]
    );
    assert!(
        !MENU.iter().any(|row| matches!(
            row,
            Row::Item {
                when: When::Recording,
                command: crate::daemon::Command::RecordToggle,
                ..
            }
        )),
        "a row drawn while recording never starts one"
    );
}

/// What a platform draws: the record row while no recording runs and the
/// stop row while one does, every row but the install row while no
/// update is offered, the install row with the offered version while one
/// is, and each drawn row under its `MENU` id in every state, so a click
/// on a menu drawn before a state changed still sends that row's
/// command. The one test that sets the offer and the recording state:
/// the others read `MENU`, not `drawn`.
#[test]
fn the_drawn_rows_follow_the_offer_and_the_recording() {
    let labels = |rows: Vec<(usize, Drawn)>| -> Vec<(usize, String)> {
        rows.into_iter()
            .map(|(id, row)| match row {
                Drawn::Item(label) => (id, label.into_owned()),
                Drawn::Separator => (id, "---".into()),
            })
            .collect()
    };
    let plain: Vec<(usize, String)> = [
        (1, "New Screenshot"),
        (2, RECORD),
        (4, "Library"),
        (5, "Settings"),
        (7, "---"),
        (8, "Quit"),
    ]
    .into_iter()
    .map(|(id, label)| (id, label.to_string()))
    .collect();
    let recording = |rows: &[(usize, String)]| -> Vec<(usize, String)> {
        let mut rows = rows.to_vec();
        rows[1] = (3, "Stop Recording".to_string());
        rows
    };

    set_offer(None);
    set_recording(false);
    assert_eq!(labels(drawn()), plain, "no offer, no recording");

    set_recording(true);
    assert_eq!(labels(drawn()), recording(&plain), "a recording");
    assert_eq!(
        debug(command(3)),
        debug(Some(crate::daemon::Command::RecordStop))
    );
    set_recording(true);
    assert_eq!(labels(drawn()), recording(&plain), "a second start");

    let version = semver::Version::parse("2.1.0-beta.3").unwrap();
    set_offer(Some(&version));
    let mut offered = plain.clone();
    offered.insert(4, (6, "Install iris 2.1.0-beta.3".to_string()));
    assert_eq!(
        labels(drawn()),
        recording(&offered),
        "an offer during a recording"
    );

    set_recording(false);
    assert_eq!(labels(drawn()), offered, "an offer of {version}");
    assert_eq!(
        debug(command(6)),
        debug(Some(crate::daemon::Command::InstallUpdate))
    );

    let newer = semver::Version::parse("2.1.0").unwrap();
    set_offer(Some(&newer));
    offered[4].1 = "Install iris 2.1.0".to_string();
    assert_eq!(labels(drawn()), offered, "a newer offer replaces the label");

    set_offer(None);
    assert_eq!(labels(drawn()), plain, "an offer withdrawn");
}

/// The usage chapter's tray table lists the item labels in menu order. A
/// row's first cell names the item in bold, once per platform label.
#[test]
fn the_usage_chapter_lists_every_item_in_order() {
    let doc = include_str!("../../../../../docs/usage.md");
    let table = doc
        .split_once("The tray menu holds the same actions:")
        .expect("usage.md introduces the tray table")
        .1;
    let documented: Vec<Vec<&str>> = table
        .lines()
        .skip_while(|l| !l.starts_with('|'))
        .take_while(|l| l.starts_with('|'))
        .filter_map(|l| {
            let cell = l.split('|').nth(1)?;
            let labels: Vec<&str> = cell.split("**").skip(1).step_by(2).collect();
            (!labels.is_empty()).then_some(labels)
        })
        .collect();
    let shipped: Vec<&str> = MENU
        .iter()
        .filter_map(|row| match row {
            Row::Item { label, .. } => Some(*label),
            Row::Update => Some(UPDATE_LABEL),
            Row::Separator => None,
        })
        .collect();
    assert_eq!(documented.len(), shipped.len(), "{documented:?}");
    for (labels, label) in documented.iter().zip(&shipped) {
        assert!(labels.contains(label), "{label:?} is not in {labels:?}");
    }
}

// WHY: the class closed here is "the tray glyph reaches the panel with
// its channels swapped, jagged, or at one size the panel rescales". A
// StatusNotifierItem pixmap is ARGB32 in network byte order; RGBA bytes
// in it read as a red-tinted glyph with alpha taken from blue. Every
// glyph decodes square at a size of its own, every covered pixel reads
// white in ARGB order, and the edges hold partial coverage. Not covered:
// how a panel scales or tints it.
#[cfg(target_os = "linux")]
#[test]
fn every_tray_pixmap_is_a_white_antialiased_argb_glyph() {
    let icons = super::linux::pixmaps();
    let sizes: Vec<i32> = icons.iter().map(|icon| icon.width).collect();
    assert_eq!(sizes, [16, 22, 24, 32, 48]);
    for icon in &icons {
        let n = icon.width;
        assert_eq!(icon.height, n);
        assert_eq!(icon.data.len(), (n * n * 4) as usize, "{n} px");
        let (px, _) = icon.data.as_chunks::<4>();
        assert!(
            px.iter().all(|p| p[0] == 0 || p[1..] == [255, 255, 255]),
            "{n} px: a covered pixel is not white in ARGB order"
        );
        let solid = px.iter().filter(|p| p[0] == 255).count();
        let edge = px.iter().filter(|p| p[0] > 0 && p[0] < 255).count();
        assert!(solid > 0 && edge > 0, "{n} px: {solid} solid, {edge} edge");
    }
}

// WHY: the class closed here is "the tray glyph sits off center". The
// glyph has sixfold symmetry about its center, so a centered raster
// equals its own half turn. The rasters were drawn with an ellipse that
// rounds an integer box out by a pixel to the right and bottom, which
// set the 16, 24, and 48 px glyphs and the menu bar template 1/20 px
// right of and below center. Every `tray-*.png` in packaging/icons is
// held, so a raster added there is covered. Not covered: the app icon,
// whose shading is not symmetric.
#[test]
fn every_tray_glyph_raster_is_centered() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/icons");
    let mut held = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !(name.starts_with("tray-") && name.ends_with(".png")) {
            continue;
        }
        let img = image::open(&path).unwrap().into_rgba8();
        let (w, h) = img.dimensions();
        for (x, y, p) in img.enumerate_pixels() {
            let turned = img.get_pixel(w - 1 - x, h - 1 - y);
            assert!(
                p[3].abs_diff(turned[3]) <= 1,
                "{name}: ({x}, {y}) has coverage {} and its half turn {}",
                p[3],
                turned[3]
            );
        }
        held.push(name);
    }
    held.sort();
    assert_eq!(
        held,
        [
            "tray-16.png",
            "tray-22.png",
            "tray-24.png",
            "tray-32.png",
            "tray-48.png",
            "tray-template.png"
        ]
    );
}

// WHY: the class closed here is "the tray glyph vanishes on a light
// panel". The pixmaps are white, which reads only on a dark panel, so
// the item also names a symbolic glyph for the panel to fill with its
// own foreground color. GTK panels and GNOME Shell fill every `rect`,
// `circle`, and `path` of a `-symbolic` icon through a stylesheet, and
// Plasma replaces the icon's `current-color-scheme` stylesheet. A part
// drawn as a stroke, as an element that stylesheet does not select, or
// in a literal color stays white on a light panel. The cases render the
// shipped SVG under both recolorings and hold every covered pixel to
// the panel's color, hold its shape to the 16 px pixmap's, and hold the
// file to the place the item points the panel at. Not covered: a
// panel's own lookup; tests/tray.rs reads the name and path the live
// item serves.
#[cfg(target_os = "linux")]
mod symbolic {
    use std::path::Path;

    const SVG: &str = include_str!("../../../../../packaging/icons/tray-symbolic.svg");

    /// `svg` at `size` px, as premultiplied RGBA.
    fn render(svg: &str, size: u32) -> Vec<[u8; 4]> {
        use resvg::{tiny_skia, usvg};
        let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("the SVG parses");
        let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();
        let scale = size as f32 / tree.size().width();
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        pixmap
            .pixels()
            .iter()
            .map(|p| [p.red(), p.green(), p.blue(), p.alpha()])
            .collect()
    }

    /// The glyph as Plasma draws it: the `current-color-scheme`
    /// stylesheet's text replaced with one that sets `ColorScheme-Text`
    /// to `color`.
    fn plasma(color: &str) -> String {
        let open = SVG
            .find("<style id=\"current-color-scheme\"")
            .expect("the glyph has no current-color-scheme stylesheet for Plasma to replace");
        let body = open + SVG[open..].find('>').unwrap() + 1;
        let close = body + SVG[body..].find("</style>").unwrap();
        format!(
            "{}.ColorScheme-Text {{ color:{color}; }}{}",
            &SVG[..body],
            &SVG[close..]
        )
    }

    /// The glyph as a GTK panel or GNOME Shell draws it: under a
    /// stylesheet that fills every `rect`, `circle`, and `path` with
    /// `color`.
    fn gtk(color: &str) -> String {
        let root = SVG.find("<svg").unwrap();
        let body = root + SVG[root..].find('>').unwrap() + 1;
        format!(
            "{}<style type=\"text/css\">rect,circle,path {{ fill: {color} !important; }}</style>{}",
            &SVG[..body],
            &SVG[body..]
        )
    }

    /// Every pixel the glyph covers holds `want` at its coverage, at the
    /// sizes panels draw tray icons at, within the renderer's rounding
    /// of 2. A part in another color differs by more than that at any
    /// coverage of 16 or more: white over `#232629` by 14.
    fn assert_drawn_in(svg: &str, want: [u8; 3], host: &str) {
        for size in [16, 22, 24, 32] {
            let px = render(svg, size);
            let covered: Vec<&[u8; 4]> = px.iter().filter(|p| p[3] >= 16).collect();
            assert!(
                covered.len() > 20,
                "{host} {size} px: {} pixels drawn",
                covered.len()
            );
            for p in covered {
                let a = u32::from(p[3]);
                let off = (0..3)
                    .map(|i| u32::from(p[i]).abs_diff((u32::from(want[i]) * a + 127) / 255))
                    .max()
                    .unwrap();
                assert!(
                    off <= 2,
                    "{host} {size} px: a pixel reads {p:?} premultiplied, not {want:?}"
                );
            }
        }
    }

    #[test]
    fn the_symbolic_glyph_takes_the_panel_color_under_gtk_and_plasma() {
        assert_drawn_in(
            &plasma("#232629"),
            [0x23, 0x26, 0x29],
            "Plasma, light panel",
        );
        assert_drawn_in(&plasma("#fcfcfc"), [0xfc, 0xfc, 0xfc], "Plasma, dark panel");
        assert_drawn_in(&gtk("#2e3436"), [0x2e, 0x34, 0x36], "GTK, light panel");
        assert_drawn_in(&gtk("#eeeeec"), [0xee, 0xee, 0xec], "GTK, dark panel");
        // A panel that recolors nothing draws it as the pixmaps.
        assert_drawn_in(SVG, [255, 255, 255], "no recoloring");
    }

    #[test]
    fn the_symbolic_glyph_has_the_16_px_pixmap_shape() {
        let drawn = render(SVG, 16);
        let icons = super::super::linux::pixmaps();
        let pixmap = icons.iter().find(|icon| icon.width == 16).unwrap();
        let (argb, _) = pixmap.data.as_chunks::<4>();
        let diffs: Vec<u32> = drawn
            .iter()
            .zip(argb)
            .map(|(p, q)| u32::from(p[3].abs_diff(q[0])))
            .collect();
        let max = diffs.iter().max().unwrap();
        let mean = diffs.iter().sum::<u32>() as f64 / diffs.len() as f64;
        assert!(
            *max <= 32 && mean <= 4.0,
            "the symbolic glyph and the 16 px pixmap differ by up to {max} in coverage, \
             {mean:.2} on average"
        );
    }

    #[test]
    fn the_symbolic_glyph_is_written_where_the_item_points_the_panel() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("cache/tray");
        let theme = super::super::linux::symbolic_theme(&dir).unwrap();
        assert_eq!(Path::new(&theme), dir, "IconThemePath");
        let file = dir.join("iris-tray-symbolic.svg");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), SVG);
        assert_eq!(super::super::linux::SYMBOLIC_NAME, "iris-tray-symbolic");

        // A glyph an earlier version wrote is replaced.
        std::fs::write(&file, "<svg xmlns=\"http://www.w3.org/2000/svg\"/>").unwrap();
        super::super::linux::symbolic_theme(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), SVG);
    }
}
