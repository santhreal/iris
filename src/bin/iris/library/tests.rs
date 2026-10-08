use std::path::PathBuf;
use std::prelude::v1::test;

use iris_lib::library::CaptureEntry;
use iris_lib::time::days_from_civil;

use super::context_menu::{place, MENU_H};
use super::entries::trash_status;
use super::keys::arrow;
use super::layout::{
    day_and_time, day_title, sections, Day, Layout, Row, Section, Step, BOTTOM_PAD, HEADER_H,
    PAD_X, ROW_GAP,
};
use super::listing::{ListPass, Listing};
use super::preview::fit;
use super::render::help_rows;
use super::{CARD_H, CARD_W, GAP, MIN_SIZE};
use crate::widgets::MENU_W;

/// Sections of `sizes[d]` captures on day `TODAY - d`, and their layout
/// at `width`.
fn grid(sizes: &[usize], width: f32) -> (Vec<Section>, Layout) {
    let days: Vec<Day> = sizes
        .iter()
        .enumerate()
        .flat_map(|(d, &n)| std::iter::repeat_n(TODAY - d as Day, n))
        .collect();
    let s = sections(&days, TODAY);
    let layout = Layout::new(&s, width);
    (s, layout)
}

/// Thursday, October 8, 2026.
const TODAY: Day = 20_734;

// WHY: the grid groups captures under a title per local day. Closed
// here: a title that names the wrong weekday or date, a recent day that
// is not "Today" or "Yesterday", a year left off a date from another
// year (or printed on this year's), and a listing split into sections
// that are not runs of one day (a day shown twice, or two days merged).
// Not covered: the platform's zone lookup behind each capture's day.
#[test]
fn day_titles_name_today_yesterday_then_the_date() {
    assert_eq!(days_from_civil(2026, 10, 8), TODAY);
    assert_eq!(day_title(TODAY, TODAY), "Today");
    assert_eq!(day_title(TODAY - 1, TODAY), "Yesterday");
    assert_eq!(day_title(TODAY - 2, TODAY), "Tuesday, October 6");
    assert_eq!(day_title(TODAY - 7, TODAY), "Thursday, October 1");
    let new_year = days_from_civil(2026, 1, 1);
    assert_eq!(day_title(new_year, TODAY), "Thursday, January 1");
    assert_eq!(
        day_title(new_year - 1, TODAY),
        "Wednesday, December 31, 2025"
    );
    assert_eq!(
        day_title(days_from_civil(2024, 2, 29), TODAY),
        "Thursday, February 29, 2024"
    );
    // A capture dated after today (the clock moved back) shows its date.
    assert_eq!(day_title(TODAY + 1, TODAY), "Friday, October 9");
    // Every seventh day has the same weekday, past "Yesterday".
    for day in TODAY - 800..TODAY - 8 {
        let weekday = |d: Day| day_title(d, TODAY).split(',').next().unwrap().to_owned();
        assert_eq!(weekday(day), weekday(day + 7), "day {day}");
    }
}

#[test]
fn sections_are_the_runs_of_one_day() {
    let days = [
        TODAY,
        TODAY,
        TODAY - 1,
        TODAY - 3,
        TODAY - 3,
        TODAY - 3,
        TODAY - 400,
    ];
    let s = sections(&days, TODAY);
    let spans: Vec<_> = s.iter().map(|s| (s.start, s.end, s.day)).collect();
    assert_eq!(
        spans,
        [
            (0, 2, TODAY),
            (2, 3, TODAY - 1),
            (3, 6, TODAY - 3),
            (6, 7, TODAY - 400)
        ]
    );
    let titles: Vec<_> = s.iter().map(|s| s.title.to_string()).collect();
    assert_eq!(
        titles,
        [
            "Today",
            "Yesterday",
            "Monday, October 5",
            "Wednesday, September 3, 2025"
        ]
    );
    assert!(sections(&[], TODAY).is_empty());
}

// WHY: render, the rubber band, keyboard navigation, scroll-into-view,
// and the editor's open morph all read card positions from one Layout.
// Closed here: two cards drawn over each other, a card out of listing
// order, a day whose first card does not start a row under its title, a
// grid that leaves a column's room unused or passes the window's padding,
// and a content height that clips the last row or scrolls past it. Every
// width and section shape below is checked against these invariants.
// Not covered: the paint of the rows.
#[test]
fn every_card_has_its_own_place_in_reading_order() {
    let shapes: [&[usize]; 7] = [
        &[],
        &[1],
        &[4],
        &[5, 1, 9],
        &[1, 1, 1, 1],
        &[37, 2],
        &[8, 8],
    ];
    let span = |c: usize| c as f32 * CARD_W + (c as f32 - 1.0) * GAP;
    for width in [100.0, MIN_SIZE.width.into(), 700.0, 960.0, 1234.5, 2560.0] {
        for sizes in shapes {
            let (secs, l) = grid(sizes, width);
            let n: usize = sizes.iter().sum();
            assert!(l.cols >= 1);
            if l.cols > 1 {
                assert!(span(l.cols) + 2.0 * PAD_X <= width, "{width}: {}", l.cols);
            }
            assert!(
                span(l.cols + 1) + 2.0 * PAD_X > width,
                "{width}: {}",
                l.cols
            );
            let rects: Vec<_> = (0..n).map(|i| l.card_rect(i).unwrap()).collect();
            assert_eq!(l.card_rect(n), None);
            for (i, &(x, y, w, h)) in rects.iter().enumerate() {
                assert_eq!((w, h), (CARD_W, CARD_H));
                assert!(x >= 0.0);
                if width >= CARD_W + 2.0 * PAD_X {
                    assert!(
                        x >= PAD_X - 0.5 && x + w <= width - PAD_X + 0.5,
                        "{width}: {x}"
                    );
                }
                if let Some(&(px_, py, ..)) = i.checked_sub(1).map(|p| &rects[p]) {
                    assert!(
                        y > py || (y == py && x > px_),
                        "card {i} before card {}",
                        i - 1
                    );
                }
                for &(bx, by, ..) in &rects[i + 1..] {
                    if by == y {
                        assert!((bx - x).abs() >= CARD_W + GAP, "{width} {sizes:?}");
                    } else {
                        assert!((by - y).abs() >= CARD_H + ROW_GAP, "{width} {sizes:?}");
                    }
                }
            }
            for (si, s) in secs.iter().enumerate() {
                let (x, y, ..) = rects[s.start];
                assert_eq!(x, l.x0, "section {si} starts a row");
                let header = l.rows.iter().position(|r| *r == Row::Header(si)).unwrap();
                assert_eq!(
                    l.tops[header] + HEADER_H,
                    y,
                    "section {si} is under its title"
                );
            }
            let bottom = rects.iter().map(|r| r.1 + r.3).fold(0.0, f32::max);
            assert_eq!(l.height, if n == 0 { 0.0 } else { bottom + BOTTOM_PAD });
        }
    }
}

// WHY: the grid builds only the rows rows_in returns. Closed here: a row
// in the viewport that is not built (a blank band in the grid, the bug a
// wheel motion between the offset and its target used to show), and a
// range that builds the whole document. Render passes the span from the
// scroll offset to the wheel target plus the viewport, so covering any
// span covers every frame of the motion. Not covered: the offsets the
// scroll handle reports.
#[test]
fn rows_in_builds_every_row_in_the_viewport_and_little_else() {
    let (_, l) = grid(&[7, 1, 12, 3, 30], 960.0);
    let rows = l.rows.len();
    let meets =
        |r: usize, top: f32, bottom: f32| l.tops[r] < bottom && l.tops[r] + l.row_h(r) > top;
    for view_h in [0.0, 1.0, 200.0, 640.0, 5000.0] {
        let mut top = -50.0;
        while top < l.height + 100.0 {
            let bottom = top + view_h;
            let (first, last) = l.rows_in(top, bottom).unwrap();
            assert!(first <= last && last < rows);
            let seen = (0..rows).filter(|&r| meets(r, top, bottom)).count();
            for r in (0..rows).filter(|&r| meets(r, top, bottom)) {
                assert!(
                    first <= r && r <= last,
                    "{top}+{view_h}: row {r} not in {first}..={last}"
                );
            }
            assert!(
                last - first < seen.max(1) + 2,
                "{top}+{view_h}: {first}..={last}"
            );
            top += 7.3;
        }
    }
    assert_eq!(Layout::default().rows_in(0.0, 100.0), None);
}

#[test]
fn entries_of_rows_are_the_cards_in_them() {
    // 4 columns: rows are H0, [0..3], H1, [3..7], [7..8].
    let (_, l) = grid(&[3, 5], 960.0);
    assert_eq!(l.cols, 4);
    assert_eq!(l.entries_of(0, 4), (0, 8));
    assert_eq!(l.entries_of(0, 0), (0, 0));
    assert_eq!(l.entries_of(1, 2), (0, 3));
    assert_eq!(l.entries_of(2, 3), (3, 7));
    assert_eq!(l.entries_of(4, 99), (7, 8));
    assert_eq!(l.entries_of(99, 120), (0, 0));
}

// WHY: arrow keys move through the grid as it is drawn, across day
// sections whose last rows are short. Closed here: a Left or Right that
// skips or repeats a card at a row or day boundary, an Up or Down that
// lands in a column a short row does not have (out of bounds) or jumps
// a row, a move off the grid's edge that wraps, and a Shift move that
// loses its anchor. Not covered: the key event dispatch.
#[test]
fn arrow_steps_follow_the_drawn_grid() {
    // 4 columns: rows are H0, [0 1 2], H1, [3 4 5 6], [7].
    let (_, l) = grid(&[3, 5], 960.0);
    let cases = [
        (0, Step::Left, 0),
        (2, Step::Right, 3),
        (3, Step::Left, 2),
        (7, Step::Right, 7),
        (1, Step::Down, 4),
        (2, Step::Down, 5),
        (6, Step::Down, 7),
        (7, Step::Down, 7),
        (7, Step::Up, 3),
        (5, Step::Up, 2),
        (6, Step::Up, 2),
        (0, Step::Up, 0),
        (4, Step::Down, 7),
    ];
    for (from, step, to) in cases {
        assert_eq!(l.step(from, step), to, "{from} {step:?}");
    }
    // Left then Right walks every card once, in listing order.
    let walk: Vec<_> = std::iter::successors(Some(0), |&i| {
        let next = l.step(i, Step::Right);
        (next != i).then_some(next)
    })
    .collect();
    assert_eq!(walk, (0..8).collect::<Vec<_>>());
    for i in 0..8 {
        for step in [Step::Left, Step::Right, Step::Up, Step::Down] {
            assert!(l.step(i, step) < 8);
        }
    }
}

#[test]
fn shift_arrows_grow_the_selection_from_its_anchor() {
    let (_, l) = grid(&[3, 5], 960.0);
    assert_eq!(arrow(&l, 0, None, None, Step::Right, false), None);
    // No card under the cursor: the first card.
    assert_eq!(
        arrow(&l, 8, None, None, Step::Down, false),
        Some((0, 0, (0, 0)))
    );
    assert_eq!(
        arrow(&l, 8, Some(99), None, Step::Right, false),
        Some((0, 0, (0, 0)))
    );
    // A plain move selects the card it lands on.
    assert_eq!(
        arrow(&l, 8, Some(1), Some(0), Step::Down, false),
        Some((4, 4, (4, 4)))
    );
    // Shift grows from the anchor, either way.
    assert_eq!(
        arrow(&l, 8, Some(1), Some(0), Step::Down, true),
        Some((4, 0, (0, 4)))
    );
    assert_eq!(
        arrow(&l, 8, Some(5), Some(5), Step::Up, true),
        Some((2, 5, (2, 5)))
    );
    // An anchor a refresh left past the end falls back to the cursor.
    assert_eq!(
        arrow(&l, 8, Some(1), Some(99), Step::Right, true),
        Some((2, 1, (1, 2)))
    );
}

#[test]
fn a_card_in_a_days_first_row_reveals_with_its_title() {
    let (_, l) = grid(&[3, 5], 960.0);
    let (_, y3, ..) = l.card_rect(3).unwrap();
    assert_eq!(l.reveal_span(3), Some((y3 - HEADER_H, y3 + CARD_H)));
    let (_, y7, ..) = l.card_rect(7).unwrap();
    assert_eq!(l.reveal_span(7), Some((y7, y7 + CARD_H)));
    assert_eq!(l.reveal_span(8), None);
}

// WHY: the rubber band selects the cards it touches. Closed here: a
// card under the band left out, a card the band only passes near
// selected, and a band across a day title missing the next day's cards.
#[test]
fn the_band_selects_the_cards_it_touches() {
    let (_, l) = grid(&[3, 5], 960.0);
    let (x, y, w, h) = l.card_rect(4).unwrap();
    assert_eq!(l.cards_in(x, y, x + w, y + h), [4]);
    assert_eq!(
        l.cards_in(x + w - 1.0, y + h - 1.0, x + w + 1.0, y + h + 1.0),
        [4]
    );
    // The gap between two cards.
    assert!(l
        .cards_in(x + w + 1.0, y, x + w + GAP - 1.0, y + h)
        .is_empty());
    // From the first day's second card down across the second day's title.
    let (x1, y1, ..) = l.card_rect(1).unwrap();
    assert_eq!(l.cards_in(x1, y1, x1 + 1.0, y + 1.0), [1, 4]);
}

fn entry(file: &str, created_ms: i64) -> CaptureEntry {
    CaptureEntry {
        path: PathBuf::from("shots").join(file),
        thumb: PathBuf::new(),
        width: 1920,
        height: 1080,
        created_ms,
    }
}

fn shot(file: &str) -> PathBuf {
    PathBuf::from("shots").join(file)
}

/// 12:00 UTC on `day`, plus `mins` minutes: the same local day in every
/// zone for any `mins` under 120, and a different one 48 hours away.
fn at(day: Day, mins: i64) -> i64 {
    (day * 86_400 + 12 * 3600 + mins * 60) * 1000
}

// WHY: the toolbar count, the captions, the day sections, the selection,
// and the thumbnail cache all derive from the shown listing, and
// Listing::set is the one way a listing is shown. Closed here: a derived
// value that disagrees with the listing, a store answer out of time order
// shown unsorted (one day split into two sections), a selection naming a
// capture no longer shown, a capture re-saved in place whose old thumbnail
// survives, an empty state shown before the store answers, and titles
// that stay "Today" past midnight. Not covered: the render code that
// reads these values.
#[test]
fn a_listing_is_unknown_until_the_store_answers() {
    let mut listing = Listing::default();
    assert_eq!(listing.listed(), None);
    assert_eq!(*listing.count(), "");
    // An empty store is an answer: the empty state shows from here on.
    assert_eq!(
        listing.set(Vec::new(), &mut Vec::new(), TODAY),
        Some(Vec::new())
    );
    assert!(listing.listed().is_some());
    assert_eq!(*listing.count(), "0 captures");
    // The same answer again is not a new listing.
    let generation = listing.generation();
    assert_eq!(listing.set(Vec::new(), &mut Vec::new(), TODAY), None);
    assert_eq!(listing.generation(), generation);
}

#[test]
fn the_count_captions_and_sections_follow_the_listing() {
    let mut listing = Listing::default();
    for (n, count) in [
        (1, "1 capture"),
        (2, "2 captures"),
        (48, "48 captures"),
        (0, "0 captures"),
    ] {
        // Two captures a day, given oldest first: the store's order is
        // not the grid's.
        let fresh: Vec<_> = (0..n)
            .map(|i| {
                let day = TODAY - 2 * ((n - 1 - i) / 2) as Day;
                entry(&format!("{i}.png"), at(day, i as i64 % 2))
            })
            .collect();
        let generation = listing.generation();
        assert!(listing.set(fresh.clone(), &mut Vec::new(), TODAY).is_some());
        assert!(listing.generation() > generation);
        assert_eq!(*listing.count(), count);
        let shown = listing.entries();
        assert_eq!(shown.len(), n);
        assert!(shown.windows(2).all(|w| w[0].created_ms >= w[1].created_ms));
        for (e, c) in shown.iter().zip(listing.captions()) {
            let (_, time) = day_and_time(e.created_ms);
            assert_eq!(c.time, time);
            assert_eq!(c.dims, "1920 × 1080");
            assert_eq!(c.name, e.path.file_name().unwrap().to_str().unwrap());
        }
        // The sections partition the listing into runs of one local day.
        let secs = listing.sections();
        assert_eq!(secs.first().map_or(0, |s| s.start), 0);
        assert_eq!(secs.last().map_or(0, |s| s.end), n);
        for w in secs.windows(2) {
            assert_eq!(w[0].end, w[1].start);
            assert!(w[0].day > w[1].day);
        }
        for s in secs {
            for e in &shown[s.start..s.end] {
                assert_eq!(day_and_time(e.created_ms).0, s.day);
            }
        }
        assert_eq!(secs.len(), n.div_ceil(2));
    }
}

#[test]
fn a_new_listing_prunes_the_selection_and_names_resaved_captures() {
    let mut listing = Listing::default();
    let first = vec![entry("c.png", 3), entry("b.png", 2), entry("a.png", 1)];
    listing.set(first, &mut Vec::new(), TODAY);
    let mut selected = vec![shot("c.png"), shot("a.png"), shot("b.png")];
    // b.png trashed, a.png re-saved in place, d.png new.
    let second = vec![entry("d.png", 4), entry("a.png", 5), entry("c.png", 3)];
    assert_eq!(
        listing.set(second, &mut selected, TODAY),
        Some(vec![shot("a.png")])
    );
    assert_eq!(selected, [shot("c.png"), shot("a.png")]);
    // The same paths with one re-saved again: still a new listing.
    let third = vec![entry("d.png", 4), entry("a.png", 6), entry("c.png", 3)];
    assert_eq!(
        listing.set(third, &mut selected, TODAY),
        Some(vec![shot("a.png")])
    );
    assert_eq!(selected, [shot("c.png"), shot("a.png")]);
}

#[test]
fn titles_move_on_at_midnight() {
    let mut listing = Listing::default();
    let fresh = vec![
        entry("b.png", at(TODAY, 0)),
        entry("a.png", at(TODAY - 2, 0)),
    ];
    let today = day_and_time(at(TODAY, 0)).0;
    listing.set(fresh.clone(), &mut Vec::new(), today);
    assert_eq!(listing.sections()[0].title, "Today");
    let generation = listing.generation();
    assert!(!listing.relabel(today), "the same day changes nothing");
    assert_eq!(listing.generation(), generation);
    assert!(listing.relabel(today + 1));
    assert_eq!(listing.sections()[0].title, "Yesterday");
    assert!(listing.generation() > generation, "the layout is rebuilt");
    // An unchanged store answer the next day relabels too.
    let mut listing = Listing::default();
    listing.set(fresh.clone(), &mut Vec::new(), today);
    assert_eq!(listing.set(fresh, &mut Vec::new(), today + 1), None);
    assert_eq!(listing.sections()[0].title, "Yesterday");
}

// WHY: a capture another program removes leaves the grid because the
// window watches every folder that holds a listed capture. Closed here:
// a listed capture whose folder goes unwatched, a folder watched twice,
// and a folder still watched after its last capture left the listing.
// Not covered: the watch itself (sys::watch's tests) and the window's
// re-watch as the folders change (tests/library.rs).
#[test]
fn the_watched_folders_are_those_of_the_listed_captures() {
    let at_dir = |dir: &str, file: &str, created_ms| CaptureEntry {
        path: PathBuf::from(dir).join(file),
        ..entry(file, created_ms)
    };
    let mut listing = Listing::default();
    assert!(listing.folders().is_empty());
    let fresh = vec![
        at_dir("b", "4.png", 4),
        at_dir("a", "3.png", 3),
        at_dir("b", "2.png", 2),
        at_dir("a/sub", "1.png", 1),
    ];
    listing.set(fresh, &mut Vec::new(), TODAY);
    assert_eq!(listing.folders(), ["a", "a/sub", "b"].map(PathBuf::from));
    listing.set(vec![at_dir("b", "4.png", 4)], &mut Vec::new(), TODAY);
    assert_eq!(listing.folders(), [PathBuf::from("b")]);
    listing.set(Vec::new(), &mut Vec::new(), TODAY);
    assert!(listing.folders().is_empty());
}

// WHY: a capture saved in this process shows in an open library window
// at once. Closed here: a store write that lands while a list pass is in
// flight and gets no pass after it (the pass in flight may have read the
// store before the write, so the window shows the stale listing until
// the next poll), a write that queues more than one pass, a poll that
// queues one, and overlapping passes. Not covered: the daemon's
// delivery of the write to the window.
#[test]
fn a_write_during_a_list_pass_queues_exactly_one_more() {
    let mut pass = ListPass::default();
    assert!(pass.begin(false), "an idle window lists at once");
    assert!(!pass.begin(false), "passes never overlap");
    assert!(!pass.begin(true));
    assert!(!pass.begin(true));
    assert!(!pass.begin(false));
    assert!(pass.land(), "a write during the pass queues one more");
    assert!(pass.begin(false), "the queued pass starts");
    assert!(!pass.land(), "two writes during a pass queue one pass");
    assert!(pass.begin(true), "a write to an idle window lists at once");
    assert!(!pass.begin(false));
    assert!(!pass.land(), "a poll during a pass queues nothing");
    assert!(pass.begin(false), "a landed pass leaves the window idle");
}

// WHY: a trash that leaves captures behind reports it; one that moves
// them all reports nothing over the grid that already shows the result.
// Closed here: a silent partial failure, and a count that disagrees with
// the captures that stayed.
#[test]
fn a_trash_reports_the_captures_that_stayed() {
    let why = "Could not move a.png to the Trash: permission denied".to_owned();
    assert_eq!(trash_status(1, &[]), None);
    assert_eq!(trash_status(5, &[]), None);
    assert_eq!(
        trash_status(1, std::slice::from_ref(&why)),
        Some(why.clone())
    );
    assert_eq!(
        trash_status(3, &[why.clone(), "Could not move b.png".into()]),
        Some(format!("2 of 3 captures stayed. {why}"))
    );
    assert_eq!(
        trash_status(4, std::slice::from_ref(&why)),
        Some(format!("1 of 4 captures stayed. {why}"))
    );
}

// WHY: the context menu opens at the pointer and stays in the window.
// Closed here: a menu cut off at the right or bottom edge, and a menu
// moved away from a pointer that has room for it. Every pointer position
// across windows from the minimum size up is checked.
#[test]
fn the_context_menu_opens_at_the_pointer_inside_the_window() {
    let (min_w, min_h): (f32, f32) = (MIN_SIZE.width.into(), MIN_SIZE.height.into());
    for (w, h) in [(min_w, min_h), (960.0, 640.0), (2560.0, 1440.0)] {
        let mut px_ = 0.0;
        while px_ <= w {
            let mut py = 0.0;
            while py <= h {
                let (x, y) = place((px_, py), w, h);
                assert!(x >= 0.0 && x + MENU_W <= w, "{w}x{h} at {px_},{py}: x {x}");
                assert!(y >= 0.0 && y + MENU_H <= h, "{w}x{h} at {px_},{py}: y {y}");
                if px_ >= 4.0 && px_ + MENU_W + 4.0 <= w {
                    assert_eq!(x, px_);
                }
                if py >= 4.0 && py + MENU_H + 4.0 <= h {
                    assert_eq!(y, py);
                }
                py += 13.0;
            }
            px_ += 17.0;
        }
    }
}

// WHY: the preview shows a capture at its own size, fitted to the
// window. Closed here: a small capture scaled up and blurred, a large one
// that passes the window, and a fitted image stretched out of its aspect.
#[test]
fn the_preview_fits_a_capture_and_never_scales_it_up() {
    for scale in [1.0, 1.5, 2.0] {
        for (w, h) in [
            (1, 1),
            (320, 200),
            (1920, 1080),
            (7680, 4320),
            (100, 5000),
            (5000, 100),
        ] {
            for (aw, ah) in [(576.0, 300.0), (896.0, 500.0), (2400.0, 1300.0)] {
                let (fw, fh) = fit(w, h, scale, aw, ah);
                let (nw, nh) = (w as f32 / scale, h as f32 / scale);
                assert!(
                    fw <= nw && fh <= nh,
                    "{w}x{h}@{scale} in {aw}x{ah}: {fw}x{fh}"
                );
                assert!(
                    fw <= aw && fh <= ah,
                    "{w}x{h}@{scale} in {aw}x{ah}: {fw}x{fh}"
                );
                // Fills the room it has along one axis, or shows whole.
                assert!(fw >= (nw.min(aw) - 1.0).min(nw * ah / nh - 1.0), "{fw}");
                // The aspect holds to the pixel the floor drops.
                assert!(
                    (fw * nh - fh * nw).abs() <= nw.max(nh),
                    "{fw}x{fh} of {nw}x{nh}"
                );
            }
        }
    }
    assert_eq!(fit(1920, 1080, 1.0, 0.0, 0.0), (0.0, 0.0));
}

// WHY: the class closed here is "the shortcuts sheet shows a global
// hotkey the daemon does not register": the record row printed
// Ctrl+Shift+R whatever `record_hotkey` held. Both global hotkey rows
// read the config; defaults would hide a hard-coded row, so the config
// here holds none. Not covered: the sheet's paint.
#[test]
fn the_shortcuts_sheet_shows_the_configured_global_hotkeys() {
    let cfg = iris_lib::config::Config {
        capture_hotkey: "Super+P".into(),
        record_hotkey: "Ctrl+Shift+5".into(),
        ..Default::default()
    };
    let rows = help_rows(&cfg);
    assert_eq!(rows[0], ("New screenshot", "Super+P".into()));
    assert_eq!(rows[1], ("Record window", "Ctrl+Shift+5".into()));
}
