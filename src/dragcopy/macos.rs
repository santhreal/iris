//! File copy on macOS: `NSURL` file URLs written to the general
//! `NSPasteboard`. Finder pastes the files themselves; text fields
//! receive the paths.

use std::path::PathBuf;

use crate::objc::{class, nsstring, sel, send, send1, with_autorelease_pool, Bool, Id};

/// Replace the general pasteboard's contents with `paths` (absolute).
pub(super) fn copy_abs_paths(paths: &[PathBuf]) -> Result<(), String> {
    with_autorelease_pool(|| unsafe {
        let pb: Id = send(class(c"NSPasteboard"), sel(c"generalPasteboard"));
        if pb.is_null() {
            return Err("NSPasteboard unavailable".to_string());
        }
        let urls: Id = send(class(c"NSMutableArray"), sel(c"array"));
        for p in paths {
            let s = nsstring(&p.to_string_lossy());
            let url: Id = send1(class(c"NSURL"), sel(c"fileURLWithPath:"), s);
            // nsstring returns +1; the URL holds its own reference.
            send::<()>(s, sel(c"release"));
            if url.is_null() {
                return Err(format!("cannot form a file URL for {}", p.display()));
            }
            send1::<Id, ()>(urls, sel(c"addObject:"), url);
        }
        let _: isize = send(pb, sel(c"clearContents"));
        let ok: Bool = send1(pb, sel(c"writeObjects:"), urls);
        if ok == 0 {
            return Err("NSPasteboard writeObjects: failed".to_string());
        }
        Ok(())
    })
}
