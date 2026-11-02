//! The synthesized camera shutter: two short impacts, the curtains
//! opening and closing, rendered once into 16-bit mono PCM. Each impact
//! is band-passed noise over three damped resonances and a low body:
//! four fifths of the energy lies from 3 to 12 kHz, as in a recorded
//! shutter, over a faint thump under 300 Hz.

use std::f32::consts::TAU;
use std::sync::LazyLock;

/// PipeWire's and most sound cards' native rate: no resampling.
pub const RATE: u32 = 48_000;
/// Rendered length. The fade-out ends it on silence, so the player
/// never steps from a live sample to nothing.
const LEN_MS: f32 = 110.0;
const FADE_MS: f32 = 12.0;
/// Peak level as a fraction of full scale: -3 dBFS. With the sound's
/// high crest, its loudness falls between a volume-change tick and a
/// message alert of the freedesktop theme.
const PEAK: f32 = 0.708;
/// The rise of a resonance and of the noise: a step onset reads as a
/// digital click, not a mechanical one.
const ATTACK_MS: f32 = 0.4;
/// The slower rise of the low body.
const BODY_ATTACK_MS: f32 = 1.5;

/// One impact, from `at_ms`.
struct Click {
    at_ms: f32,
    gain: f32,
    /// Noise through a band-pass: decay (ms), centre (Hz), Q, gain.
    hiss: (f32, f32, f32, f32),
    /// Damped resonances: frequency (Hz), decay (ms), gain.
    modes: [(f32, f32, f32); 3],
    /// The low thump: frequency (Hz), decay (ms), gain.
    body: (f32, f32, f32),
}

/// The first curtain, then the second 48 ms later, a little lower and
/// louder.
const CLICKS: [Click; 2] = [
    Click {
        at_ms: 0.5,
        gain: 0.85,
        hiss: (2.2, 7_500.0, 0.7, 3.0),
        modes: [
            (4_300.0, 3.0, 0.25),
            (6_800.0, 2.2, 0.3),
            (9_100.0, 1.5, 0.2),
        ],
        body: (170.0, 7.0, 0.12),
    },
    Click {
        at_ms: 48.5,
        gain: 1.0,
        hiss: (3.0, 6_200.0, 0.7, 3.0),
        modes: [
            (3_700.0, 3.5, 0.25),
            (5_900.0, 2.6, 0.3),
            (8_000.0, 1.8, 0.2),
        ],
        body: (140.0, 9.0, 0.15),
    },
];

fn samples(ms: f32) -> usize {
    (ms * RATE as f32 / 1000.0) as usize
}

/// Seconds since an onset `t` samples ago.
fn secs(t: usize) -> f32 {
    t as f32 / RATE as f32
}

/// White noise in [-1, 1] from xorshift32: deterministic, so every run
/// renders the same sound and the tests pin it.
fn noise(n: usize) -> Vec<f32> {
    let mut seed: u32 = 0x9E37_79B9;
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        })
        .collect()
}

/// A band-pass biquad (constant 0 dB peak gain) at `hz` with quality
/// `q`, run over `x`.
fn band_pass(x: &[f32], hz: f32, q: f32) -> impl Iterator<Item = f32> + '_ {
    let w = TAU * hz / RATE as f32;
    let alpha = w.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let (b0, b2) = (alpha / a0, -alpha / a0);
    let (a1, a2) = (-2.0 * w.cos() / a0, (1.0 - alpha) / a0);
    let (mut x1, mut x2, mut y1, mut y2) = (0f32, 0f32, 0f32, 0f32);
    x.iter().map(move |&v| {
        let y = b0 * v + b2 * x2 - a1 * y1 - a2 * y2;
        (x2, x1, y2, y1) = (x1, v, y1, y);
        y
    })
}

/// A damped sine at `hz` decaying over `decay_ms`, `t` samples after its
/// onset, rising over `attack_ms`.
fn ring(t: usize, hz: f32, decay_ms: f32, attack_ms: f32) -> f32 {
    let s = secs(t);
    (TAU * hz * s).sin() * (-s * 1000.0 / decay_ms).exp() * (s * 1000.0 / attack_ms).min(1.0)
}

/// The shutter as PCM samples, peak-normalized to `PEAK`.
fn render() -> Vec<i16> {
    let len = samples(LEN_MS);
    let mut out = vec![0f32; len];
    let noise = noise(len);
    for c in &CLICKS {
        let start = samples(c.at_ms);
        let (decay_ms, hz, q, hiss_gain) = c.hiss;
        let hiss = band_pass(&noise[start..], hz, q);
        for (t, (s, n)) in out[start..].iter_mut().zip(hiss).enumerate() {
            let rise = (secs(t) * 1000.0 / ATTACK_MS).min(1.0);
            let mut v = hiss_gain * n * (-secs(t) * 1000.0 / decay_ms).exp() * rise;
            for (hz, decay_ms, g) in c.modes {
                v += g * ring(t, hz, decay_ms, ATTACK_MS);
            }
            let (hz, decay_ms, g) = c.body;
            v += g * ring(t, hz, decay_ms, BODY_ATTACK_MS);
            *s += c.gain * v;
        }
    }
    let fade = samples(FADE_MS);
    for (k, s) in out.iter_mut().rev().take(fade).enumerate() {
        let f = k as f32 / fade as f32;
        *s *= f * f;
    }
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    let scale = PEAK * f32::from(i16::MAX) / peak;
    out.iter().map(|s| (s * scale).round() as i16).collect()
}

/// The shutter's samples, rendered on first use.
pub fn pcm() -> &'static [i16] {
    static PCM: LazyLock<Vec<i16>> = LazyLock::new(render);
    &PCM
}

/// The shutter as a RIFF WAVE image: the 44-byte PCM header, then the
/// samples little-endian.
pub fn wav() -> Vec<u8> {
    let pcm = pcm();
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

// WHY: these close the class "the shutter is late, long, or loud" for
// the sound itself: the first impact sounds within a millisecond of the
// buffer's start, the second follows it as the louder, both are 60 dB
// under the peak by 100 ms, and the peak sits at -3 dBFS without
// clipping. Not covered: how it sounds, and the delay of the player
// that sends it to a device.
#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(b: &[u8], i: usize) -> u16 {
        u16::from_le_bytes([b[i], b[i + 1]])
    }

    fn u32_at(b: &[u8], i: usize) -> u32 {
        u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
    }

    /// `PlaySoundW` with `SND_NODEFAULT` plays nothing for an image it
    /// cannot parse, so a header field that drifts from the samples
    /// silences every capture with no error anywhere.
    #[test]
    fn header_describes_the_samples() {
        let w = wav();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32_at(&w, 4) as usize, w.len() - 8);
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32_at(&w, 16), 16, "fmt chunk size");
        assert_eq!(u16_at(&w, 20), 1, "PCM");
        assert_eq!(u16_at(&w, 22), 1, "mono");
        assert_eq!(u32_at(&w, 24), RATE);
        assert_eq!(u32_at(&w, 28), RATE * 2, "bytes per second");
        assert_eq!(u16_at(&w, 32), 2, "bytes per frame");
        assert_eq!(u16_at(&w, 34), 16, "bits per sample");
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32_at(&w, 40) as usize, w.len() - 44);
        assert_eq!(w.len() - 44, samples(LEN_MS) * 2);
    }

    /// The peak sits at -3 dBFS without clipping, and the sound starts
    /// and ends on digital silence: a live first or last sample pops.
    #[test]
    fn level_is_bounded_and_edges_are_silent() {
        let pcm = pcm();
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let dbfs = 20.0 * (f32::from(peak) / f32::from(i16::MAX)).log10();
        assert!((dbfs + 3.0).abs() < 0.05, "peak {dbfs} dBFS");
        assert_eq!(pcm.first(), Some(&0));
        assert_eq!(pcm.last(), Some(&0));
    }

    /// The first impact is audible within a millisecond, the second
    /// lands 48 ms after it, 2 to 6 dB louder, and both are over 60 dB
    /// under the peak by 100 ms.
    #[test]
    fn the_impacts_start_at_once_and_end_inside_100_ms() {
        let pcm = pcm();
        let peak = f32::from(pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0));
        let level = |range: std::ops::Range<f32>| {
            pcm[samples(range.start)..samples(range.end)]
                .iter()
                .map(|s| f32::from(s.unsigned_abs()) / peak)
                .fold(0f32, f32::max)
        };
        let db = |v: f32| 20.0 * v.log10();
        assert!(db(level(0.0..1.0)) > -12.0, "onset {}", db(level(0.0..1.0)));
        // Just before the second impact the first has died away.
        assert!(
            db(level(46.0..48.0)) < -30.0,
            "gap {}",
            db(level(46.0..48.0))
        );
        let (first, second) = (level(0.0..20.0), level(48.0..68.0));
        let rise = db(second) - db(first);
        assert!((2.0..6.0).contains(&rise), "second over first {rise} dB");
        assert!(
            db(level(100.0..LEN_MS)) < -60.0,
            "tail {}",
            db(level(100.0..LEN_MS))
        );
    }
}
