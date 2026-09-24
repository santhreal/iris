//! The ScreenCast portal session behind a Wayland recording: the
//! compositor's own picker selects the window, and the session hands
//! over a PipeWire remote and the stream's node.
//!
//! The session runs on its own thread for its whole life. Dropping the
//! `Portal` closes it, which ends the compositor's screencast and its
//! sharing indicator. A close from the compositor's side (the window
//! closed, or sharing stopped from the indicator) sets `closed`. The
//! thread rings the recording's doorbell after the pick's answer, after
//! a close from the compositor, and when it exits.

use std::os::fd::OwnedFd;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::{PersistMode, ResponseError, Session};
use enumflags2::BitFlags;
use futures::channel::oneshot;
use futures::future::{select, Either};
use futures::StreamExt;

use crate::record::wake::Wake;
use crate::record::{Doorbell, CANCELLED_PREFIX};

type Picked = Result<(OwnedFd, u32), String>;

pub(crate) struct Portal {
    /// The PipeWire node of the picked window's stream.
    pub(crate) node: u32,
    /// Set once the compositor closed the session.
    pub(crate) closed: Arc<AtomicBool>,
    /// Dropped with the portal: the session thread then closes it.
    _close: oneshot::Sender<()>,
}

impl Portal {
    /// Open a session and let the compositor's picker select the
    /// window. Blocks while the picker is up; a stop meanwhile ends
    /// the pick as a cancel and dismisses the picker. Returns the
    /// session and the PipeWire remote to connect to. `wake` is the
    /// recording's, installed on `bell`.
    pub(crate) fn open(
        stop: &Receiver<()>,
        bell: &Doorbell,
        wake: &Wake,
    ) -> Result<(Self, OwnedFd), String> {
        let (picked, rx) = mpsc::channel();
        let (close, close_rx) = oneshot::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let flag = closed.clone();
        let bell = bell.clone();
        std::thread::Builder::new()
            .name("iris-portal".into())
            .spawn(move || {
                // Dropped after the session, however it ends: `open`
                // then reads the dropped channel instead of sleeping on.
                let exit = RingOnExit(bell);
                futures::executor::block_on(session(picked, flag, close_rx, &exit.0))
            })
            .map_err(|e| format!("spawn portal thread: {e}"))?;
        // The answer and the stop each ring the wake; nothing else
        // ends the wait.
        loop {
            // Drained before the channels are read: a send after the
            // read rings again, and the next wait ends at once.
            wake.drain();
            match rx.try_recv() {
                Ok(result) => {
                    let (fd, node) = result?;
                    let portal = Self {
                        node,
                        closed,
                        _close: close,
                    };
                    return Ok((portal, fd));
                }
                Err(TryRecvError::Disconnected) => {
                    return Err("the portal thread exited without an answer".to_string())
                }
                Err(TryRecvError::Empty) => {}
            }
            // A stop is a send OR a disconnect.
            if !matches!(stop.try_recv(), Err(TryRecvError::Empty)) {
                return Err(format!("{CANCELLED_PREFIX} stopped during source pick"));
            }
            wake.wait(None, None);
        }
    }
}

/// Rings the doorbell when dropped.
struct RingOnExit(Doorbell);

impl Drop for RingOnExit {
    fn drop(&mut self) {
        self.0.ring();
    }
}

/// The session thread: create, pick, report on `picked`, then hold the
/// session until the recording drops its end of `close` or the
/// compositor closes it, and close it. `bell` wakes the recording
/// after the answer and after a close from the compositor.
async fn session(
    picked: mpsc::Sender<Picked>,
    closed: Arc<AtomicBool>,
    close: oneshot::Receiver<()>,
    bell: &Doorbell,
) {
    let mut close = close;
    let screencast = match Screencast::new().await {
        Ok(s) => s,
        Err(e) => {
            let _ = picked.send(Err(format!(
                "portal ScreenCast unavailable (is xdg-desktop-portal running?): {e}"
            )));
            return;
        }
    };
    let session = match screencast.create_session(Default::default()).await {
        Ok(s) => s,
        Err(e) => {
            let _ = picked.send(Err(format!("portal session create failed: {e}")));
            return;
        }
    };
    // Subscribed before the pick, so a close during it is not missed.
    let ended = session.receive_closed().await;
    let result = match select(pin!(pick(&screencast, &session)), &mut close).await {
        Either::Left((result, _)) => result,
        // The recording stopped during the pick: closing the session
        // dismisses the picker.
        Either::Right(_) => {
            let _ = session.close().await;
            return;
        }
    };
    let ok = result.is_ok();
    let sent = picked.send(result).is_ok();
    bell.ring();
    if sent && ok {
        match ended {
            Ok(ended) => {
                let mut ended = pin!(ended);
                if let Either::Left(_) = select(ended.next(), close).await {
                    crate::ilog!("iris: record: the compositor closed the screencast session");
                    closed.store(true, Ordering::Release);
                    bell.ring();
                }
            }
            Err(_) => {
                let _ = close.await;
            }
        }
    }
    // Closing a session the compositor already closed fails harmlessly.
    let _ = session.close().await;
}

/// Select one window, start the session (the compositor shows its
/// picker), and open the PipeWire remote.
async fn pick(screencast: &Screencast, session: &Session<Screencast>) -> Picked {
    let options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from(SourceType::Window))
        .set_cursor_mode(CursorMode::Hidden)
        .set_multiple(false)
        .set_persist_mode(PersistMode::DoNot);
    screencast
        .select_sources(session, options)
        .await
        .and_then(|request| request.response())
        .map_err(|e| format!("portal select_sources failed: {e}"))?;
    let streams = screencast
        .start(session, None, Default::default())
        .await
        .and_then(|request| request.response())
        .map_err(|e| match e {
            ashpd::Error::Response(ResponseError::Cancelled) => {
                format!("{CANCELLED_PREFIX} portal dialog dismissed")
            }
            e => format!("portal start denied or failed: {e}"),
        })?;
    let node = streams
        .streams()
        .first()
        .ok_or("portal returned no streams")?
        .pipe_wire_node_id();
    let fd = screencast
        .open_pipe_wire_remote(session, Default::default())
        .await
        .map_err(|e| format!("portal PipeWire remote failed: {e}"))?;
    Ok((fd, node))
}
