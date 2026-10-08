//! Start at login, and the Updates pane's version row: the running
//! version, the update state, Check Now, and Install.

use std::time::SystemTime;

use gpui::*;

use super::Settings;
use crate::update::UpdateInfo;
use crate::{theme, widgets};

/// What the version row offers.
#[derive(Debug)]
pub(crate) enum Release {
    /// No check has found a newer release.
    None,
    /// The newer release a check found. Install shows.
    Found(UpdateInfo),
    /// Install is downloading the release or has handed it off. Check
    /// and Install hide, and a check already running changes nothing.
    Installing,
}

/// The release the last check offered for the configured channel, as
/// the window opens with it.
pub(super) fn offered_release() -> Release {
    match crate::update::offered() {
        Some(info) => Release::Found(info),
        None => Release::None,
    }
}

/// "Last checked today at 14:05" or "Last checked on 2026-10-07 at
/// 14:05", in local time.
pub(super) fn checked_line(at: SystemTime, now: SystemTime) -> String {
    let secs = |t: SystemTime| {
        t.duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    };
    let (y, mo, d, h, mi, _) = iris_lib::time::local_fields(secs(at));
    let (ny, nmo, nd, ..) = iris_lib::time::local_fields(secs(now));
    if (y, mo, d) == (ny, nmo, nd) {
        format!("Last checked today at {h:02}:{mi:02}")
    } else {
        format!("Last checked on {y:04}-{mo:02}-{d:02} at {h:02}:{mi:02}")
    }
}

impl Settings {
    /// Read when the last check ran (a small state file): on open and
    /// after a check, never per frame.
    pub(super) fn with_last_checked(mut self) -> Self {
        self.last_checked = crate::update::last_checked();
        self
    }

    /// Point this account's start-at-login entry (`sys::autostart`) at
    /// the toggle when the two differ.
    pub(super) fn store_login(&self) -> Result<(), String> {
        if self.login == crate::sys::autostart::enabled() {
            return Ok(());
        }
        crate::sys::autostart::set(self.login)
    }

    /// The version row's state line.
    fn update_line(&self) -> String {
        if let Some(note) = &self.update_note {
            return note.clone();
        }
        if let Release::Found(info) = &self.release {
            return format!("Version {} is available", info.version);
        }
        match self.last_checked {
            Some(at) => checked_line(at, SystemTime::now()),
            None => "Not checked yet".to_string(),
        }
    }

    /// Check GitHub for a newer release on a background thread and show
    /// the result on the version row. The network call never touches
    /// the UI loop.
    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        if self.checking || matches!(self.release, Release::Installing) {
            return;
        }
        self.checking = true;
        self.update_note = Some("Checking\u{2026}".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (result, at) = cx
                .background_executor()
                .spawn(async move { (crate::update::check(), crate::update::last_checked()) })
                .await;
            this.update(cx, |this, cx| {
                this.checking = false;
                this.last_checked = at;
                this.checked(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show a check's `result`. A newer release shows Install. While
    /// Install runs, a check that finishes changes nothing.
    pub(super) fn checked(&mut self, result: Result<Option<UpdateInfo>, String>) {
        if matches!(self.release, Release::Installing) {
            return;
        }
        let (note, release) = match result {
            Ok(Some(info)) => (
                format!("Version {} is available", info.version),
                Release::Found(info),
            ),
            Ok(None) => ("iris is up to date".to_string(), Release::None),
            Err(e) => (e, Release::None),
        };
        self.update_note = Some(note);
        self.release = release;
    }

    /// Download and verify the release a check found, off the UI loop,
    /// then hand it to a detached `iris --update`, which stops this
    /// daemon, installs it, and starts the new iris.
    pub(super) fn install_update(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.start_install() else {
            return;
        };
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (info, result) = cx
                .background_executor()
                .spawn(async move {
                    let result = crate::update::hand_off(&info);
                    (info, result)
                })
                .await;
            this.update(cx, |this, cx| {
                this.installed(info, result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// The release to install, when a check found one. Check and
    /// Install hide until the download ends.
    pub(super) fn start_install(&mut self) -> Option<UpdateInfo> {
        match std::mem::replace(&mut self.release, Release::Installing) {
            Release::Found(info) => {
                self.update_note = Some(format!("Downloading {}\u{2026}", info.version));
                Some(info)
            }
            other => {
                self.release = other;
                None
            }
        }
    }

    /// Show how the download of `info` ended. A failed one shows its
    /// error and offers Install again.
    pub(super) fn installed(&mut self, info: UpdateInfo, result: Result<(), String>) {
        self.update_note = Some(match result {
            Ok(()) => format!("Installing {}; iris restarts when it is done", info.version),
            Err(e) => {
                self.release = Release::Found(info);
                e
            }
        });
    }

    /// The version row: the running version over the update state, and
    /// Install when a check found a newer release, Check Now otherwise.
    /// Neither shows while a check or an install runs.
    pub(super) fn version_row(&self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let action = match &self.release {
            _ if self.checking => None,
            Release::Installing => None,
            Release::Found(info) => Some(
                widgets::push_button(
                    "install-update",
                    format!("Install {}", info.version),
                    widgets::ButtonStyle::Primary,
                )
                .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
            ),
            Release::None => Some(
                widgets::push_button("check-update", "Check Now", widgets::ButtonStyle::Bordered)
                    .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx))),
            ),
        };
        div()
            .h(px(theme::ROW_H_TALL))
            .px(px(theme::ROW_PAD_X))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(theme::TEXT_BODY))
                            .line_height(px(16.))
                            .text_color(theme::FG)
                            .font_features(theme::tabular())
                            .child(concat!("iris ", env!("CARGO_PKG_VERSION"))),
                    )
                    .child(
                        div()
                            .text_size(px(theme::TEXT_SMALL))
                            .line_height(px(14.))
                            .text_color(theme::FG_DIM)
                            .font_features(theme::tabular())
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.update_line()),
                    ),
            )
            .children(action)
    }
}
