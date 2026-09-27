//! The Startup and Updates sections: start at login, the version, and
//! the update check and install.

use gpui::*;

use super::Settings;
use crate::theme;
use crate::update::UpdateInfo;

/// What the Updates section offers.
#[derive(Debug)]
pub(crate) enum Release {
    /// No check has found a newer release.
    None,
    /// The newer release the last check found. Install shows.
    Found(UpdateInfo),
    /// Install is downloading the release or has handed it off. Check
    /// and Install hide, and a check already running changes nothing.
    Installing,
}

impl Settings {
    /// The Startup and Updates sections.
    pub(super) fn app_sections(&self, cx: &mut Context<Self>) -> [Div; 2] {
        let startup = self.section(
            "Startup",
            vec![self
                .toggle_row(
                    "tog-login",
                    "Start at login",
                    self.login,
                    cx.listener(|this, _, _, cx| {
                        this.login = !this.login;
                        this.open_dropdown = None;
                        cx.notify();
                    }),
                )
                .into_any_element()],
        );

        // Current version, a manual check, and Install once a check
        // finds a newer release. Both run off the UI loop and report
        // through the status pill. Install goes left of the
        // right-aligned Check, so Check stays under the pointer that
        // clicked it and a second click checks again.
        let mut release = div().flex().gap(px(8.));
        if let Release::Found(_) = self.release {
            release = release.child(
                crate::widgets::button("install-update", "Install update", false)
                    .on_click(cx.listener(|this, _, _, cx| this.install_update(cx))),
            );
        }
        if !matches!(self.release, Release::Installing) {
            release = release.child(
                crate::widgets::button("check-update", "Check for updates", false)
                    .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx))),
            );
        }
        let updates = self.section(
            "Updates",
            vec![
                self.row_shell(
                    "Version",
                    div()
                        .text_size(px(theme::TEXT_BODY))
                        .text_color(theme::FG_DIM)
                        .child(env!("CARGO_PKG_VERSION")),
                )
                .into_any_element(),
                self.row_shell("Latest release", release).into_any_element(),
            ],
        );
        [startup, updates]
    }

    /// Point this account's start-at-login entry (`sys::autostart`) at
    /// the toggle when the two differ.
    pub(super) fn store_login(&self) -> Result<(), String> {
        if self.login == crate::sys::autostart::enabled() {
            return Ok(());
        }
        crate::sys::autostart::set(self.login)
    }

    /// Check GitHub for a newer release on a background thread and
    /// report the result in the status pill. The network call never
    /// touches the UI loop.
    pub(super) fn check_updates(&mut self, cx: &mut Context<Self>) {
        self.status = Some("checking…".to_string());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { crate::update::check() })
                .await;
            this.update(cx, |this, cx| {
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
        let (status, release) = match result {
            Ok(Some(info)) => (
                format!("update available: {}", info.version),
                Release::Found(info),
            ),
            Ok(None) => ("up to date".to_string(), Release::None),
            Err(e) => (e, Release::None),
        };
        self.status = Some(status);
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
                self.status = Some(format!("downloading {}…", info.version));
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
        self.status = Some(match result {
            Ok(()) => format!("installing {}; iris restarts when it is done", info.version),
            Err(e) => {
                self.release = Release::Found(info);
                e
            }
        });
    }
}
