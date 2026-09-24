//! Process allocator policy, applied once at startup.
//!
//! glibc raises its mmap threshold each time an mmapped block is freed,
//! so after the first capture every frame-sized buffer (grab, crop,
//! encode) is carved from a malloc arena instead, and each worker
//! thread's arena keeps its freed copies resident. A long-running daemon
//! then grows by megabytes per capture. Pinning the threshold keeps
//! blocks of `MMAP_THRESHOLD` and up on mmap, which returns them to the
//! kernel on free. The macOS and Windows allocators already return
//! large blocks on free and need no policy.

/// Blocks at least this large bypass the arenas. Frame buffers are
/// megabytes; the recorder recycles its frames, so no hot path pays the
/// per-allocation mmap.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const MMAP_THRESHOLD: libc::c_int = 256 * 1024;

/// Apply the policy. Call from `main` before any other thread starts.
pub fn tune() {
    // SAFETY: mallopt only sets an allocator parameter, and no other
    // thread exists yet to race the change.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, MMAP_THRESHOLD);
    }
}
