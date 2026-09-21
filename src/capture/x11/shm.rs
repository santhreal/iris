use x11rb::connection::Connection;
use x11rb::protocol::xproto::ImageFormat;

use super::*;

/// The capture path's persistent MIT-SHM segment, keyed on size and
/// reused across grabs: shmget+shmat+attach+detach+shmdt was five
/// syscalls and an X round trip on every capture. The segment is
/// marked IPC_RMID at creation, so it dies with the process on any
/// exit path.
struct CaptureShm {
    addr: *mut u8,
    seg: u32,
    size: usize,
}

// The segment is only ever touched through this mutex; the raw
// pointer never crosses threads unsynchronized.
unsafe impl Send for CaptureShm {}

fn capture_shm() -> &'static parking_lot::Mutex<Option<CaptureShm>> {
    static SHM: std::sync::LazyLock<parking_lot::Mutex<Option<CaptureShm>>> =
        std::sync::LazyLock::new(|| parking_lot::Mutex::new(None));
    &SHM
}

/// Whether the server speaks MIT-SHM at all, queried once per process:
/// the version cannot change over a connection's life.
pub(crate) fn shm_supported() -> bool {
    use x11rb::protocol::shm::ConnectionExt as ShmExt;
    static VERDICT: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        shared_conn()
            .ok()
            .and_then(|(conn, _)| {
                ShmExt::shm_query_version(conn)
                    .ok()
                    .and_then(|c| c.reply().ok())
            })
            .map(|v| v.major_version >= 1)
            .unwrap_or(false)
    });
    *VERDICT
}

pub(super) fn try_shm_grab_into<C>(
    conn: &C,
    root: x11rb::protocol::xproto::Window,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
    out: &mut [u8],
) -> Option<u8>
where
    C: Connection + x11rb::protocol::xproto::ConnectionExt,
{
    use x11rb::protocol::shm::ConnectionExt as ShmExt;
    if !shm_supported() {
        return None;
    }
    let size = width as usize * height as usize * 4;
    let mut guard = capture_shm().lock();
    // Re-key on size: a monitor hotplug or a different union rect
    // needs a fresh segment.
    if guard.as_ref().map(|s| s.size) != Some(size) {
        if let Some(old) = guard.take() {
            unsafe { libc::shmdt(old.addr as *const _) };
            let _ = ShmExt::shm_detach(conn, old.seg);
        }
        *guard = (|| -> Option<CaptureShm> {
            unsafe {
                let shmid = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
                if shmid < 0 {
                    return None;
                }
                // Mark for removal now: the segment dies with the last
                // detach, so no crash path can leak it.
                libc::shmctl(shmid, libc::IPC_RMID, std::ptr::null_mut());
                let addr = libc::shmat(shmid, std::ptr::null(), 0);
                if addr as isize == -1 {
                    return None;
                }
                let seg = conn.generate_id().ok()?;
                if ShmExt::shm_attach(conn, seg, shmid as u32, false)
                    .ok()
                    .and_then(|c| c.check().ok())
                    .is_none()
                {
                    libc::shmdt(addr);
                    return None;
                }
                Some(CaptureShm {
                    addr: addr as *mut u8,
                    seg,
                    size,
                })
            }
        })();
    }
    let s = guard.as_ref()?;
    let reply = ShmExt::shm_get_image(
        conn,
        root,
        x,
        y,
        width,
        height,
        !0u32,
        ImageFormat::Z_PIXMAP.into(),
        s.seg,
        0,
    )
    .ok()?
    .reply()
    .ok()?;
    if reply.depth == 24 {
        let src = unsafe { std::slice::from_raw_parts(s.addr as *const u8, s.size) };
        let pixels = width as usize * height as usize;
        let bpp = src.len() / pixels.max(1);
        convert_to_rgba(src, pixels, bpp, out, u32::from(width), u32::from(height)).ok()?;
    }
    Some(reply.depth)
}
