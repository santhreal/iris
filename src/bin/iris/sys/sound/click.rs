//! A synthesized camera shutter for an OS that ships none: two damped
//! clicks, the curtains opening and closing, rendered once into a 16-bit
//! mono PCM WAV image.

use std::f32::consts::TAU;

const RATE: u32 = 44_100;
/// Rendered length. The fade-out ends it on silence, so the player
/// never steps from a live sample to nothing.
const LEN_MS: f32 = 110.0;
const FADE_MS: f32 = 10.0;
/// Corner of the one-pole low-pass on the noise. A recorded shutter's
/// transients centre near 7 kHz; white noise alone centres near 11.
const NOISE_LP_HZ: f32 = 8_000.0;
/// Peak level as a fraction of full scale: -6 dBFS.
const PEAK: f32 = 0.5;

/// One click: a noise transient over a low body thump, both decaying
/// exponentially from `at_ms`.
struct Click {
    at_ms: f32,
    gain: f32,
    decay_ms: f32,
    body_hz: f32,
    body_gain: f32,
    body_decay_ms: f32,
}

const CLICKS: [Click; 2] = [
    Click {
        at_ms: 1.0,
        gain: 1.0,
        decay_ms: 1.8,
        body_hz: 180.0,
        body_gain: 0.15,
        body_decay_ms: 3.0,
    },
    Click {
        at_ms: 62.0,
        gain: 0.8,
        decay_ms: 2.6,
        body_hz: 140.0,
        body_gain: 0.15,
        body_decay_ms: 4.0,
    },
];

fn samples(ms: f32) -> usize {
    (ms * RATE as f32 / 1000.0) as usize
}

/// The shutter as PCM samples, peak-normalized to `PEAK`.
fn render() -> Vec<i16> {
    let mut out = vec![0f32; samples(LEN_MS)];
    // xorshift32: deterministic noise, so every run renders the same
    // sound and the tests pin it.
    let mut seed: u32 = 0x9E37_79B9;
    let lp_coeff = 1.0 - (-TAU * NOISE_LP_HZ / RATE as f32).exp();
    for c in &CLICKS {
        let tau = samples(c.decay_ms) as f32;
        let body_tau = samples(c.body_decay_ms) as f32;
        let w = TAU * c.body_hz / RATE as f32;
        let mut lp = 0f32;
        for (t, s) in out[samples(c.at_ms)..].iter_mut().enumerate() {
            let t = t as f32;
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            lp += lp_coeff * (noise - lp);
            let body = c.body_gain * (w * t).sin() * (-t / body_tau).exp();
            *s += c.gain * (lp * (-t / tau).exp() + body);
        }
    }
    let fade = samples(FADE_MS);
    for (k, s) in out.iter_mut().rev().take(fade).enumerate() {
        *s *= k as f32 / fade as f32;
    }
    let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    let scale = PEAK * f32::from(i16::MAX) / peak;
    out.iter().map(|s| (s * scale).round() as i16).collect()
}

/// The shutter as a RIFF WAVE image: the 44-byte PCM header, then the
/// samples little-endian.
pub fn wav() -> Vec<u8> {
    let pcm = render();
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

    /// The peak sits at -6 dBFS without clipping, and the sound starts
    /// and ends on digital silence: a live first or last sample pops.
    #[test]
    fn level_is_bounded_and_edges_are_silent() {
        let pcm = render();
        let peak = pcm.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        let target = PEAK * f32::from(i16::MAX);
        assert!((f32::from(peak) - target).abs() <= 1.0, "peak {peak}");
        let onset = samples(CLICKS[0].at_ms);
        assert!(onset > 0 && pcm[..onset].iter().all(|s| *s == 0));
        assert_eq!(pcm.last(), Some(&0));
    }
}
