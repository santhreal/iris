//! Isolate the XComposite pixmap grab path from the app: pick a window by
//! id (arg 1, decimal), grab 10 frames, print per-frame results.
//! Usage: DISPLAY=:99 record_smoke <window-id>

use x11rb::connection::Connection;
use x11rb::protocol::composite::{ConnectionExt as CompositeExt, Redirect};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, ImageFormat};

fn main() {
    let id: u32 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .expect("usage: record_smoke <window-id>");
    let (conn, _) = x11rb::connect(None).expect("connect");

    let version = conn
        .composite_query_version(0, 4)
        .expect("query")
        .reply()
        .expect("reply");
    println!(
        "composite {}.{}",
        version.major_version, version.minor_version
    );

    conn.composite_redirect_window(id, Redirect::AUTOMATIC)
        .expect("redirect")
        .check()
        .expect("redirect check");

    let geom = conn
        .get_geometry(id)
        .expect("geom")
        .reply()
        .expect("geom reply");
    println!("geometry {}x{}", geom.width, geom.height);

    for n in 0..10 {
        let t = std::time::Instant::now();
        let pixmap = conn.generate_id().expect("id");
        let named = conn
            .composite_name_window_pixmap(id, pixmap)
            .expect("name")
            .check();
        match named {
            Ok(()) => {}
            Err(e) => {
                println!("frame {n}: name_window_pixmap error: {e}");
                break;
            }
        }
        let img = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                pixmap,
                0,
                0,
                geom.width,
                geom.height,
                !0u32,
            )
            .expect("get_image")
            .reply();
        match img {
            Ok(img) => {
                let nonzero = img.data.iter().filter(|&&b| b != 0).count();
                println!(
                    "frame {n}: {} bytes, {} nonzero, depth {}, {:?}",
                    img.data.len(),
                    nonzero,
                    img.depth,
                    t.elapsed()
                );
            }
            Err(e) => {
                println!("frame {n}: get_image error: {e}");
                break;
            }
        }
        conn.free_pixmap(pixmap).expect("free");
    }
    println!("done");
}
