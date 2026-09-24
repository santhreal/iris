//! One recording from a live source: the run of Matroska segments it
//! becomes, and the output file they join into.
//!
//! The source hands every captured frame to `frame` with its capture
//! time. The first frame fixes the canvas and the codec. A frame of a
//! new size, a mic toggle, or a pause closes the open segment. A resume
//! or a mic toggle opens the next one with the closed segment's last
//! frame, so a source that stays still afterwards still covers the
//! time; otherwise the next frame opens it. `finish` waits for every
//! segment and joins them into the output.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::child::Closing;
use super::codec::{canvas_for, VideoCodec};
use super::encoder::{Encoder, SegmentSpec};
use super::join::{self, segment_path};
use super::mkv::PixFmt;
use super::{RecControl, RecordingSpec};
use crate::config::{RecordingEncoder, RecordingFormat};

/// A captured frame's size and memory layout.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shape {
    pub width: u32,
    pub height: u32,
    pub pix: PixFmt,
}

/// How long a resume waits for the closed segment's last frame: its
/// writer hands it over after draining, and a wedged ffmpeg is killed
/// within the encoder's flush limit, which ends the writer too.
const HANDOFF: Duration = Duration::from_secs(11);

pub struct Recorder {
    output: PathBuf,
    fps: u32,
    format: RecordingFormat,
    encoder: RecordingEncoder,
    mic: bool,
    paused: bool,
    /// Fixed by the first frame: every segment encodes onto it.
    canvas: Option<(u32, u32)>,
    codec: Option<VideoCodec>,
    open: Option<(Encoder, Shape)>,
    /// The last frame of the segment closed most recently.
    held: Option<(Receiver<Vec<u8>>, Shape)>,
    closing: Vec<Closing>,
    spares: Vec<Vec<u8>>,
}

impl Recorder {
    pub fn new(spec: &RecordingSpec) -> Self {
        Self {
            output: spec.output.clone(),
            fps: spec.fps,
            format: spec.format,
            encoder: spec.encoder,
            mic: spec.mic && spec.format.audio_codec().is_some(),
            paused: false,
            canvas: None,
            codec: None,
            open: None,
            held: None,
            closing: Vec::new(),
            spares: Vec::new(),
        }
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// An empty buffer to capture the next frame into.
    pub fn take_buf(&mut self) -> Vec<u8> {
        match &mut self.open {
            Some((enc, _)) => enc.take_buf(),
            None => self.spares.pop().unwrap_or_default(),
        }
    }

    /// Return a buffer that was taken and not handed to `frame`.
    pub fn give_back(&mut self, mut buf: Vec<u8>) {
        match &mut self.open {
            Some((enc, _)) => enc.give_back(buf),
            None if self.spares.len() < 2 => {
                buf.clear();
                self.spares.push(buf);
            }
            None => {}
        }
    }

    /// Hand over one frame of `shape`, captured at `at`. Paused, the
    /// frame is dropped.
    pub fn frame(&mut self, buf: Vec<u8>, shape: Shape, at: Instant) -> Result<(), String> {
        if self.paused {
            self.give_back(buf);
            return Ok(());
        }
        if self.open.as_ref().is_some_and(|(_, s)| *s != shape) {
            self.close(at);
        }
        if self.open.is_none() {
            // A fresh frame opens the segment; the held one is stale.
            self.held = None;
            self.open_segment(shape, at)?;
        }
        let (enc, _) = self.open.as_mut().expect("segment is open");
        enc.write(buf, at)
    }

    /// Apply a chip control at `at`.
    pub fn apply(&mut self, ctl: RecControl, at: Instant) -> Result<(), String> {
        match ctl {
            RecControl::Pause if !self.paused => {
                self.paused = true;
                self.close(at);
                Ok(())
            }
            RecControl::Resume if self.paused => {
                self.paused = false;
                self.reopen(at)
            }
            // A format without audio has no mic to toggle.
            RecControl::ToggleMic if self.format.audio_codec().is_some() => {
                self.mic = !self.mic;
                if self.open.is_none() {
                    return Ok(());
                }
                self.close(at);
                self.reopen(at)
            }
            RecControl::Pause | RecControl::Resume | RecControl::ToggleMic => Ok(()),
        }
    }

    /// Close every segment at `at` and join them into the output.
    pub fn finish(mut self, at: Instant) -> Result<PathBuf, String> {
        self.close(at);
        self.held = None;
        join::finish(self.closing, self.format, &self.output)
    }

    /// End after `err`: the segments written so far still join into
    /// the output, and the returned error states where it is.
    pub fn fail(mut self, at: Instant, err: String) -> String {
        self.close(at);
        self.held = None;
        join::finish_after(err, self.closing, self.format, &self.output)
    }

    fn open_segment(&mut self, shape: Shape, at: Instant) -> Result<(), String> {
        let canvas = *self
            .canvas
            .get_or_insert_with(|| canvas_for(shape.width, shape.height));
        let (format, encoder) = (self.format, self.encoder);
        let codec = *self
            .codec
            .get_or_insert_with(|| VideoCodec::for_recording(format, encoder, Some(canvas)));
        let enc = Encoder::start(&SegmentSpec {
            path: segment_path(&self.output, self.closing.len()),
            width: shape.width,
            height: shape.height,
            pix: shape.pix,
            canvas,
            fps: self.fps,
            codec,
            format,
            mic: self.mic,
            epoch: at,
        })?;
        self.open = Some((enc, shape));
        Ok(())
    }

    fn close(&mut self, at: Instant) {
        if let Some((enc, shape)) = self.open.take() {
            let (closing, held) = enc.finish(at);
            self.closing.push(closing);
            self.held = Some((held, shape));
        }
    }

    /// Open the next segment with the last closed segment's frame at
    /// `at`. Nothing to reopen before the first frame.
    fn reopen(&mut self, at: Instant) -> Result<(), String> {
        let Some((held, shape)) = self.held.take() else {
            return Ok(());
        };
        let Ok(frame) = held.recv_timeout(HANDOFF) else {
            return Ok(());
        };
        self.open_segment(shape, at)?;
        let (enc, _) = self.open.as_mut().expect("segment is open");
        enc.write(frame, at)
    }
}

#[cfg(test)]
#[path = "recorder/tests.rs"]
mod tests;
