//! Device selection for desktop recording on Windows and macOS, from
//! ffmpeg's own listing (`-list_devices true` prints to stderr), so a
//! recording names a device that exists instead of guessing. Pure, so
//! testable on any host.

#[cfg(any(windows, test))]
/// First audio device in a `-f dshow -list_devices true` listing:
/// the quoted name on a line ending in `(audio)`.
pub fn dshow_first_audio(listing: &str) -> Option<String> {
    listing.lines().find_map(|l| {
        let l = l.trim_end();
        if !l.ends_with("(audio)") {
            return None;
        }
        let start = l.find('"')? + 1;
        let end = start + l[start..].find('"')?;
        Some(l[start..end].to_string())
    })
}

#[cfg(any(target_os = "macos", test))]
/// Indices in a `-f avfoundation -list_devices true` listing: every
/// `Capture screen` video device in order (screen ordinal N is
/// `screens[N]`), and the first audio device.
pub fn avfoundation_indices(listing: &str) -> (Vec<usize>, Option<usize>) {
    let mut in_audio = false;
    let (mut screens, mut audio) = (Vec::new(), None);
    for l in listing.lines() {
        if l.contains("AVFoundation audio devices") {
            in_audio = true;
            continue;
        }
        if l.contains("AVFoundation video devices") {
            in_audio = false;
            continue;
        }
        // "[AVFoundation indev @ 0x..] [N] Name": the second bracket.
        let Some(rest) = l.split_once("] [").map(|(_, r)| r) else {
            continue;
        };
        let Some((idx, name)) = rest.split_once("] ") else {
            continue;
        };
        let Ok(idx) = idx.parse::<usize>() else {
            continue;
        };
        if in_audio {
            audio = audio.or(Some(idx));
        } else if name.starts_with("Capture screen") {
            screens.push(idx);
        }
    }
    (screens, audio)
}

#[cfg(test)]
mod tests {
    // WHY: the class closed here is "recording names a device ffmpeg
    // does not have": a guessed `audio=default` or a fixed screen index
    // makes ffmpeg fail to open the input. Listings are ffmpeg 8's
    // exact stderr shapes; a changed ffmpeg format is not caught here.
    use super::*;

    const DSHOW: &str = r#"[dshow @ 0000021b0fd35b40] "Integrated Camera" (video)
[dshow @ 0000021b0fd35b40]   Alternative name "@device_pnp_\\?\usb#vid_30c9"
[dshow @ 0000021b0fd35b40] "Microphone Array (Intel® Smart Sound)" (audio)
[dshow @ 0000021b0fd35b40]   Alternative name "@device_cm_{33D9}\wave_{974A}"
[dshow @ 0000021b0fd35b40] "Line In" (audio)"#;

    #[test]
    fn dshow_picks_first_audio_name_with_inner_parens() {
        assert_eq!(
            dshow_first_audio(DSHOW).as_deref(),
            Some("Microphone Array (Intel® Smart Sound)")
        );
    }

    #[test]
    fn dshow_without_audio_is_none() {
        assert_eq!(dshow_first_audio(DSHOW.lines().next().unwrap()), None);
        assert_eq!(dshow_first_audio(""), None);
    }

    const AVF: &str = "[AVFoundation indev @ 0x1] AVFoundation video devices:
[AVFoundation indev @ 0x1] [0] FaceTime HD Camera
[AVFoundation indev @ 0x1] [1] OBS Virtual Camera
[AVFoundation indev @ 0x1] [2] Capture screen 0
[AVFoundation indev @ 0x1] [3] Capture screen 1
[AVFoundation indev @ 0x1] AVFoundation audio devices:
[AVFoundation indev @ 0x1] [0] MacBook Pro Microphone
[AVFoundation indev @ 0x1] [1] Capture screen audio";

    #[test]
    fn avfoundation_finds_screens_in_order_and_first_mic() {
        assert_eq!(avfoundation_indices(AVF), (vec![2, 3], Some(0)));
    }

    #[test]
    fn avfoundation_screen_named_audio_device_is_not_a_screen() {
        // Only the audio section holds audio; "Capture screen audio"
        // there must not become the video input.
        let only_audio = AVF
            .lines()
            .filter(|l| !l.contains("] [2]") && !l.contains("] [3]"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(avfoundation_indices(&only_audio), (vec![], Some(0)));
    }
}
