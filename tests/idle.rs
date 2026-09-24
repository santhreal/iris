//! WHY: an idle daemon does not wake on a timer. The `blocking` crate's
//! thread pool serves zbus (the tray and the portals) and async-fs, and
//! blocking 1.7 keeps every pool thread it starts for the life of the
//! process, waking it every 500 ms: each daemon woke twice a second at
//! idle. Closed here: a pool thread that outlives its work, whichever
//! blocking release the lock file resolves to. Not covered: other
//! timers in the daemon.

#[cfg(target_os = "linux")]
#[test]
fn an_idle_blocking_pool_thread_exits() {
    use std::time::{Duration, Instant};

    fn pool_threads() -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("list this process's threads")
            .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
            .filter(|comm| comm.starts_with("blocking-"))
            .count()
    }

    let ran_on = futures::executor::block_on(blocking::unblock(|| {
        std::thread::current().name().map(str::to_owned)
    }));
    assert!(
        ran_on
            .as_deref()
            .is_some_and(|n| n.starts_with("blocking-")),
        "the task ran on {ran_on:?}, not a pool thread"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool_threads() > 0 {
        assert!(
            Instant::now() < deadline,
            "a blocking pool thread is still up 5 s after its last task"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
