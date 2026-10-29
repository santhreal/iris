//! Text recognition through the Vision framework: a
//! `VNRecognizeTextRequest` (macOS 10.15 and later) over the image file,
//! with the top candidate of each recognized line.

use std::path::Path;
use std::ptr::null_mut;

use super::objc::{
    class, nsstring, sel, send, send1, send2, string, with_autorelease_pool, Bool, Id,
};

#[link(name = "Vision", kind = "framework")]
extern "C" {}

/// The text in the image at `source`, one recognized line per line, in
/// the order Vision reports them. Empty when the image holds no text.
pub(crate) fn recognize_text(source: &Path) -> Result<String, String> {
    let path = source
        .to_str()
        .ok_or_else(|| format!("path is not UTF-8: {}", source.display()))?;
    with_autorelease_pool(|| unsafe { recognize(path) })
}

unsafe fn recognize(path: &str) -> Result<String, String> {
    let request_class = class(c"VNRecognizeTextRequest");
    if request_class.is_null() {
        return Err("text recognition failed: the Vision framework is not available".to_string());
    }
    let ns_path = nsstring(path);
    let url: Id = send1(class(c"NSURL"), sel(c"fileURLWithPath:"), ns_path);
    send::<()>(ns_path, sel(c"release"));
    let options: Id = send(class(c"NSDictionary"), sel(c"dictionary"));
    let handler: Id = send2(
        send::<Id>(class(c"VNImageRequestHandler"), sel(c"alloc")),
        sel(c"initWithURL:options:"),
        url,
        options,
    );
    let request: Id = send(send::<Id>(request_class, sel(c"alloc")), sel(c"init"));
    let requests: Id = send1(class(c"NSArray"), sel(c"arrayWithObject:"), request);
    let mut error: Id = null_mut();
    let ok: Bool = send2(
        handler,
        sel(c"performRequests:error:"),
        requests,
        &mut error as *mut Id,
    );
    let text = if ok == 0 {
        let reason = string(send(error, sel(c"localizedDescription")));
        Err(format!(
            "text recognition failed: {}",
            if reason.is_empty() {
                "Vision reported no reason"
            } else {
                reason.as_str()
            }
        ))
    } else {
        Ok(lines(send(request, sel(c"results"))))
    };
    send::<()>(request, sel(c"release"));
    send::<()>(handler, sel(c"release"));
    text
}

/// The top candidate of each `VNRecognizedTextObservation` in
/// `results`, joined by newlines.
unsafe fn lines(results: Id) -> String {
    let count: usize = send(results, sel(c"count"));
    let mut text = String::new();
    for i in 0..count {
        let observation: Id = send1(results, sel(c"objectAtIndex:"), i);
        let candidates: Id = send1(observation, sel(c"topCandidates:"), 1usize);
        let best: Id = send(candidates, sel(c"firstObject"));
        let line = string(send(best, sel(c"string")));
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
    }
    text
}
