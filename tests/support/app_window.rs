//! The app window's layout, logical px, as `widgets::sidebar_frame` and
//! `library::sidebar` lay it out: the opening width, the sidebar, and
//! the sidebar rows of the settings panes. The settings page's rows sit
//! under the 52 px toolbar at the 20 px inset, with the card's right edge
//! 20 px in from the window's right edge at the opening width.

/// The window's opening width (`library::OPEN_SIZE`).
pub const OPEN_W: f32 = 920.0;
/// The sidebar's width (`theme::SIDEBAR_W`).
pub const SIDEBAR_W: f32 = 200.0;
/// The toolbar band over the sidebar (`theme::TOOLBAR_H`), a sidebar
/// row (`theme::SIDEBAR_ROW_H`), and the gap over the Settings header
/// (`theme::SIDEBAR_SECTION_GAP`).
const BAND_H: f32 = 52.0;
const ROW_H: f32 = 28.0;
const SECTION_GAP: f32 = 18.0;
/// A point on a sidebar row right of every label, where the selected
/// row's fill shows.
const FILL_X: f32 = 170.0;
/// The green level between the sidebar (`theme::SIDEBAR_BG`, 38) and
/// the selected row's fill over it (`theme::SIDEBAR_SELECTED`, 60).
const LIT: u8 = 49;

/// The window's scale while it has its opening width `w`, device px.
pub fn scale(w: usize) -> f32 {
    w as f32 / OPEN_W
}

/// The center line of settings pane `pane`'s sidebar row, in
/// `settings::Pane::ALL` order: under the band, the Library row, the
/// gap, and the Settings header.
pub fn pane_row_y(pane: usize) -> f32 {
    BAND_H + ROW_H + SECTION_GAP + ROW_H + ROW_H * pane as f32 + ROW_H / 2.0
}

/// Whether pane `pane`'s sidebar row shows the selected fill in `px`,
/// `w` device px wide at `scale`: the pane the settings page shows.
pub fn pane_shown(px: &[[u8; 3]], w: usize, scale: f32, pane: usize) -> bool {
    let y = (pane_row_y(pane) * scale) as usize;
    px[y * w + (FILL_X * scale) as usize][1] > LIT
}
