//! The doorbell a live recording rings after it sends its source a
//! control or the stop. A source that blocks on its own event loop
//! installs a ring that wakes that loop, so it reads the channels the
//! moment something arrives and sleeps without a timeout otherwise. A
//! source that reads its channels on a timeout installs no ring, and a
//! ring is then a no-op.

use std::sync::{Arc, OnceLock};

type Ring = Box<dyn Fn() + Send + Sync>;

/// Shared by a recording and its source; every clone rings one bell.
#[derive(Clone, Default)]
pub struct Doorbell(Arc<OnceLock<Ring>>);

impl Doorbell {
    /// Install the source's ring. The first install holds. A source
    /// reads its channels after installing, so a send before the
    /// install is not missed.
    pub fn install(&self, ring: impl Fn() + Send + Sync + 'static) {
        let _ = self.0.set(Box::new(ring));
    }

    /// Wake the source, when it installed a ring.
    pub fn ring(&self) {
        if let Some(ring) = self.0.get() {
            ring();
        }
    }
}

// WHY: the class closed here is "a send that does not wake a blocked
// source": a ring lost between clones, or a later install replacing
// the ring the source waits on. The recording side (every send rings)
// is covered in the recording tests; the Linux wake in `wake`.
#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counter(bell: &Doorbell) -> Arc<AtomicUsize> {
        let n = Arc::new(AtomicUsize::new(0));
        let rung = n.clone();
        bell.install(move || {
            rung.fetch_add(1, Ordering::SeqCst);
        });
        n
    }

    #[test]
    fn a_ring_before_the_install_is_a_no_op() {
        let bell = Doorbell::default();
        bell.ring();
        let n = counter(&bell);
        assert_eq!(n.load(Ordering::SeqCst), 0);
        bell.ring();
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn every_clone_rings_the_installed_ring() {
        let bell = Doorbell::default();
        let (sender, source) = (bell.clone(), bell.clone());
        let n = counter(&source);
        sender.ring();
        bell.ring();
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_second_install_keeps_the_first_ring() {
        let bell = Doorbell::default();
        let first = counter(&bell);
        let second = counter(&bell);
        bell.ring();
        assert_eq!(
            (first.load(Ordering::SeqCst), second.load(Ordering::SeqCst)),
            (1, 0)
        );
    }
}
