//! Where the library grid puts things: the day sections of a listing,
//! and the rows, card rects, and keyboard moves of those sections at a
//! window width. Render, the rubber band, keyboard navigation, and
//! scroll-into-view all read card positions from one [`Layout`].

use gpui::SharedString;
use iris_lib::time::{civil_from_days, days_from_civil};

use super::{CARD_H, CARD_W, GAP};

/// A local calendar day, as days since 1970-01-01.
pub(super) type Day = i64;

/// Horizontal inset of the grid from the window edges, at least.
pub(super) const PAD_X: f32 = 20.0;
/// A day header row: its title sits on the row's bottom, above the
/// cards.
pub(super) const HEADER_H: f32 = 44.0;
/// Space between two card rows of one day.
pub(super) const ROW_GAP: f32 = 20.0;
/// Space under the last card row, so its captions do not touch the
/// window's bottom edge.
pub(super) const BOTTOM_PAD: f32 = 24.0;

/// The local day and the "HH:MM" caption of a capture made at
/// `created_ms`.
pub(super) fn day_and_time(created_ms: i64) -> (Day, SharedString) {
    let (y, mo, d, h, mi, _) = iris_lib::time::local_fields(created_ms.div_euclid(1000));
    (
        days_from_civil(y, mo, d),
        SharedString::from(format!("{h:02}:{mi:02}")),
    )
}

/// Today, local time.
pub(super) fn today() -> Day {
    let (y, mo, d, ..) = iris_lib::time::local_now();
    days_from_civil(y, mo, d)
}

/// Seconds until the next local midnight, at least one.
pub(super) fn secs_to_midnight() -> u64 {
    let (.., h, mi, s) = iris_lib::time::local_now();
    let into_day = u64::from(h) * 3600 + u64::from(mi) * 60 + u64::from(s);
    86_400u64.saturating_sub(into_day).max(1)
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A day header's title: "Today", "Yesterday", or the weekday and date,
/// with the year when it is not `today`'s.
pub(super) fn day_title(day: Day, today: Day) -> String {
    match today - day {
        0 => "Today".to_owned(),
        1 => "Yesterday".to_owned(),
        _ => {
            // 1970-01-01 was a Thursday.
            let weekday = WEEKDAYS[(day + 4).rem_euclid(7) as usize];
            let (y, m, d) = civil_from_days(day);
            let month = MONTHS[m as usize - 1];
            if y == civil_from_days(today).0 {
                format!("{weekday}, {month} {d}")
            } else {
                format!("{weekday}, {month} {d}, {y}")
            }
        }
    }
}

/// One day's captures: entries `start..end` of the listing.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Section {
    pub start: usize,
    pub end: usize,
    pub day: Day,
    pub title: SharedString,
}

/// The sections of `days`, one per run of equal days. `days` is the
/// listing's, newest first.
pub(super) fn sections(days: &[Day], today: Day) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for (i, &day) in days.iter().enumerate() {
        match out.last_mut() {
            Some(s) if s.day == day => s.end = i + 1,
            _ => out.push(Section {
                start: i,
                end: i + 1,
                day,
                title: SharedString::from(day_title(day, today)),
            }),
        }
    }
    out
}

/// One row of the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Row {
    /// Section `0`'s header.
    Header(usize),
    /// Entries `start..end`, left to right.
    Cards { start: usize, end: usize },
}

/// A keyboard move through the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
    Left,
    Right,
    Up,
    Down,
}

/// The grid at one width: rows top to bottom, each row's top in
/// document space (0 is the top of the scrolled content), and each
/// entry's row and column.
#[derive(Default)]
pub(super) struct Layout {
    pub cols: usize,
    /// Left edge of the first column; the grid is centered.
    pub x0: f32,
    pub rows: Vec<Row>,
    pub tops: Vec<f32>,
    /// Content height, bottom padding included.
    pub height: f32,
    /// Per entry: (row index, column).
    place: Vec<(usize, usize)>,
}

impl Layout {
    pub(super) fn new(sections: &[Section], width: f32) -> Layout {
        let cols = (((width - 2.0 * PAD_X + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
        let grid_w = cols as f32 * CARD_W + (cols - 1) as f32 * GAP;
        let x0 = ((width - grid_w) / 2.0).max(0.0).round();
        let n = sections.last().map_or(0, |s| s.end);
        let mut layout = Layout {
            cols,
            x0,
            place: vec![(0, 0); n],
            ..Layout::default()
        };
        let mut y = 0.0;
        for (si, s) in sections.iter().enumerate() {
            layout.rows.push(Row::Header(si));
            layout.tops.push(y);
            y += HEADER_H;
            let mut start = s.start;
            while start < s.end {
                let end = (start + cols).min(s.end);
                let r = layout.rows.len();
                for (c, i) in (start..end).enumerate() {
                    layout.place[i] = (r, c);
                }
                layout.rows.push(Row::Cards { start, end });
                layout.tops.push(y);
                y += CARD_H;
                if end < s.end {
                    y += ROW_GAP;
                }
                start = end;
            }
        }
        layout.height = if n == 0 { 0.0 } else { y + BOTTOM_PAD };
        layout
    }

    /// Entry `i`'s card rect, (x, y, w, h) in document space.
    pub(super) fn card_rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        let &(r, c) = self.place.get(i)?;
        Some((
            self.x0 + c as f32 * (CARD_W + GAP),
            self.tops[r],
            CARD_W,
            CARD_H,
        ))
    }

    /// Row `r`'s height.
    pub(super) fn row_h(&self, r: usize) -> f32 {
        match self.rows[r] {
            Row::Header(_) => HEADER_H,
            Row::Cards { .. } => CARD_H,
        }
    }

    /// The rows that intersect document span `top..bottom`, plus one
    /// row either side, as an inclusive index range; None with no rows.
    pub(super) fn rows_in(&self, top: f32, bottom: f32) -> Option<(usize, usize)> {
        let last = self.rows.len().checked_sub(1)?;
        // The first row whose bottom is below `top`.
        let first = self
            .tops
            .partition_point(|&t| t <= top)
            .saturating_sub(1)
            .min(last);
        let first = if self.tops[first] + self.row_h(first) <= top {
            (first + 1).min(last)
        } else {
            first
        };
        // The last row whose top is above `bottom`.
        let end = self.tops.partition_point(|&t| t < bottom).max(first + 1);
        Some((first.saturating_sub(1), end.min(last)))
    }

    /// The entries whose cards intersect the document-space rect.
    pub(super) fn cards_in(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<usize> {
        (0..self.place.len())
            .filter(|&i| {
                self.card_rect(i)
                    .is_some_and(|(x, y, w, h)| x < x1 && x + w > x0 && y < y1 && y + h > y0)
            })
            .collect()
    }

    /// The entry a keyboard `step` from entry `i` lands on. Left and
    /// Right follow listing order across rows and days; Up and Down move
    /// to the nearest column of the card row above or below. A move off
    /// the grid's end stays on `i`.
    pub(super) fn step(&self, i: usize, step: Step) -> usize {
        let n = self.place.len();
        if n == 0 {
            return 0;
        }
        let i = i.min(n - 1);
        let (r, c) = self.place[i];
        let row = |r: usize| match self.rows[r] {
            Row::Cards { start, end } => Some((start, end)),
            Row::Header(_) => None,
        };
        match step {
            Step::Left => i.saturating_sub(1),
            Step::Right => (i + 1).min(n - 1),
            Step::Up => (0..r)
                .rev()
                .find_map(row)
                .map_or(i, |(start, end)| start + c.min(end - start - 1)),
            Step::Down => (r + 1..self.rows.len())
                .find_map(row)
                .map_or(i, |(start, end)| start + c.min(end - start - 1)),
        }
    }

    /// The document span to keep in view for entry `i`: its card, and
    /// its day header when the card is in the day's first row.
    pub(super) fn reveal_span(&self, i: usize) -> Option<(f32, f32)> {
        let &(r, _) = self.place.get(i)?;
        let top = match r.checked_sub(1).map(|h| self.rows[h]) {
            Some(Row::Header(_)) => self.tops[r - 1],
            _ => self.tops[r],
        };
        Some((top, self.tops[r] + CARD_H))
    }

    /// The entries in rows `first..=last`, as a half-open range; an
    /// empty range when those rows hold no cards.
    pub(super) fn entries_of(&self, first: usize, last: usize) -> (usize, usize) {
        let Some(rows) = self
            .rows
            .get(first..=last.min(self.rows.len().saturating_sub(1)))
        else {
            return (0, 0);
        };
        let cards = |r: &Row| match *r {
            Row::Cards { start, end } => Some((start, end)),
            Row::Header(_) => None,
        };
        match (
            rows.iter().find_map(cards),
            rows.iter().rev().find_map(cards),
        ) {
            (Some((start, _)), Some((_, end))) => (start, end),
            _ => (0, 0),
        }
    }
}
