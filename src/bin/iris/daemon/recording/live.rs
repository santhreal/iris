//! The daemon's live recording slot. A recording starts and ends only
//! through `set` and `take_if`, each of which shows the new state, so the
//! tray menu's record row follows every start and every end, whichever
//! path took it.

use iris_lib::record::ActiveRecording;

/// One recording at a time, and the function that shows whether one
/// runs. Called on the daemon's main thread.
pub(crate) struct Live<T = ActiveRecording> {
    slot: parking_lot::Mutex<Option<T>>,
    show: fn(bool),
}

impl<T> Live<T> {
    pub(crate) const fn new(show: fn(bool)) -> Self {
        Self {
            slot: parking_lot::Mutex::new(None),
            show,
        }
    }

    /// Whether a recording runs.
    pub(crate) fn active(&self) -> bool {
        self.slot.lock().is_some()
    }

    /// Run `f` on the live recording, or on `None`. `f` cannot start or
    /// end one.
    pub(crate) fn with<R>(&self, f: impl FnOnce(Option<&mut T>) -> R) -> R {
        f(self.slot.lock().as_mut())
    }

    /// Register `rec` as the live recording.
    pub(super) fn set(&self, rec: T) {
        *self.slot.lock() = Some(rec);
        (self.show)(true);
    }

    /// End and return the live recording when `pred` holds for it.
    pub(super) fn take_if(&self, pred: impl FnOnce(&mut T) -> bool) -> Option<T> {
        let taken = self.slot.lock().take_if(pred);
        if taken.is_some() {
            (self.show)(false);
        }
        taken
    }

    /// End and return the live recording.
    pub(super) fn take(&self) -> Option<T> {
        self.take_if(|_| true)
    }
}

#[cfg(test)]
mod tests {
    use super::Live;

    static SHOWN: parking_lot::Mutex<Vec<bool>> = parking_lot::Mutex::new(Vec::new());

    fn show(on: bool) {
        SHOWN.lock().push(on);
    }

    /// Every start and every end shows the new state once; a read, a
    /// change inside the recording, and an end that does not happen show
    /// nothing.
    #[test]
    fn every_start_and_end_shows_the_state_once() {
        let live: Live<u32> = Live::new(show);
        assert!(!live.active());
        assert_eq!(live.take(), None);
        assert_eq!(*SHOWN.lock(), [] as [bool; 0], "an end with nothing live");

        live.set(1);
        assert!(live.active());
        assert_eq!(*SHOWN.lock(), [true], "a start");

        live.with(|rec| *rec.unwrap() += 1);
        assert_eq!(live.take_if(|rec| *rec == 1), None);
        assert!(live.active());
        assert_eq!(*SHOWN.lock(), [true], "a change and a refused end");

        assert_eq!(live.take_if(|rec| *rec == 2), Some(2));
        assert!(!live.active());
        assert_eq!(*SHOWN.lock(), [true, false], "an end");

        live.set(3);
        assert_eq!(live.take(), Some(3));
        assert_eq!(live.take(), None);
        assert_eq!(*SHOWN.lock(), [true, false, true, false], "a second run");
    }
}
