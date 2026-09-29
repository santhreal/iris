//! The folders that hold the listed captures. A watch on them reports a
//! capture another program deletes or moves away, and the window reads
//! the store at once, which drops the capture from it. Where the OS
//! refuses the watch, the window reads the store every `REFRESH`.

use std::path::PathBuf;

use futures::StreamExt as _;
use gpui::Context;

use super::{Library, REFRESH};
use crate::sys::watch::{DirWatch, Entries, Waker, Woke};

/// How a library window learns of captures removed outside iris.
pub(super) enum Folders {
    /// No capture listed, so none to lose.
    Unwatched,
    /// A watch on these folders, sorted.
    Watched(Watched),
    /// The OS refused a watch: the window polls the store for its life.
    Polled,
}

/// A watch that runs on a thread of its own until this drops.
pub(super) struct Watched {
    dirs: Vec<PathBuf>,
    waker: Waker,
}

impl Drop for Watched {
    fn drop(&mut self) {
        // The thread's wait returns, and the thread ends and closes the
        // watch.
        self.waker.wake();
    }
}

impl Library {
    /// Watch the folders of the listing on show, where they are not the
    /// folders watched already.
    pub(super) fn follow_folders(&mut self, cx: &mut Context<Self>) {
        let dirs = self.listing.folders();
        match &self.folders {
            Folders::Polled => return,
            Folders::Watched(watched) if watched.dirs == dirs => return,
            Folders::Unwatched if dirs.is_empty() => return,
            _ => {}
        }
        self.folders = Folders::Unwatched;
        if dirs.is_empty() {
            return;
        }
        match watch(&dirs, cx) {
            Ok(waker) => {
                self.folders = Folders::Watched(Watched { dirs, waker });
                // A capture removed after the listing read the store and
                // before the watch began reports no change: read once more.
                self.refresh(true, cx);
            }
            Err(e) => {
                iris_lib::ilog!(
                    "iris: library: watch the capture folders: {e}; reading the library every {} ms",
                    REFRESH.as_millis()
                );
                self.folders = Folders::Polled;
                self.arm_refresh(cx);
            }
        }
    }
}

/// Watch `dirs` on a thread that ends when the returned waker is called,
/// and read the store on each change the thread reports, until the
/// window closes.
fn watch(dirs: &[PathBuf], cx: &mut Context<Library>) -> std::io::Result<Waker> {
    let watch = DirWatch::new(dirs, Entries::Removed)?;
    let waker = watch.waker();
    let (tx, mut changes) = futures::channel::mpsc::unbounded();
    std::thread::Builder::new()
        .name("library-watch".into())
        .spawn(move || loop {
            match watch.wait(None) {
                Woke::Changed => {
                    if tx.unbounded_send(()).is_err() {
                        break;
                    }
                }
                Woke::Woken => break,
                // A signal interrupted the wait.
                Woke::Limit => {}
            }
        })?;
    cx.spawn(async move |this, cx| {
        while changes.next().await.is_some() {
            if this.update(cx, |this, cx| this.refresh(true, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    Ok(waker)
}
