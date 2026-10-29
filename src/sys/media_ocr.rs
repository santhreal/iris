//! Text recognition through Windows.Media.Ocr (Windows 10 and later):
//! an `OcrEngine` for the user's profile languages over the decoded
//! image, one recognized line per line.

use std::path::Path;

use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

/// The error for a profile whose languages have no OCR recognizer.
const NO_LANGUAGE: &str = "text recognition failed: Windows has no OCR recognizer for your \
     languages; add a language with Optical character recognition in Settings > Time & \
     language > Language & region";

/// The text in the image at `source`, one recognized line per line, in
/// the order the engine reports them. Empty when the image holds no text.
pub(crate) fn recognize_text(source: &Path) -> Result<String, String> {
    let img = image::open(source).map_err(failed)?.into_rgba8();
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| NO_LANGUAGE)?;
    let img = within(img, OcrEngine::MaxImageDimension().map_err(failed)?);
    let (w, h) = img.dimensions();
    let mut bgra = img.into_raw();
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = CryptographicBuffer::CreateFromByteArray(&bgra).map_err(failed)?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Bgra8, w as i32, h as i32)
            .map_err(failed)?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|op| op.get())
        .map_err(failed)?;
    let mut text = String::new();
    for line in &result.Lines().map_err(failed)? {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line.Text().map_err(failed)?.to_string_lossy());
    }
    Ok(text)
}

fn failed(e: impl std::fmt::Display) -> String {
    format!("text recognition failed: {e}")
}

/// `img`, scaled down to fit `max` pixels on its longer side when it is
/// larger: the engine rejects a bigger image.
fn within(img: image::RgbaImage, max: u32) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= max || max == 0 {
        return img;
    }
    let scale = |n: u32| ((u64::from(n) * u64::from(max)) / u64::from(long)).max(1) as u32;
    image::imageops::resize(
        &img,
        scale(w),
        scale(h),
        image::imageops::FilterType::Triangle,
    )
}

// WHY: the engine rejects an image longer than its maximum dimension
// on either side, so a capture of a wide desktop failed outright. Not
// covered: the engine's own maximum, read at run time.
#[cfg(test)]
mod tests {
    use super::within;

    #[test]
    fn an_image_past_the_limit_scales_to_it_and_keeps_its_aspect() {
        let cases = [
            ((100, 50), 200, (100, 50)),
            ((200, 50), 200, (200, 50)),
            ((400, 100), 200, (200, 50)),
            ((100, 400), 200, (50, 200)),
            ((10_000, 3), 100, (100, 1)),
            ((120, 80), 0, (120, 80)),
        ];
        for ((w, h), max, want) in cases {
            let got = within(image::RgbaImage::new(w, h), max).dimensions();
            assert_eq!(got, want, "{w}x{h} within {max}");
        }
    }
}
