//! WHY: a parked window must receive no more fixups. A fixup restacks
//! its window, and a window manager shows a window it is asked to raise,
//! so a fixup that outlived its window's park put the parked capture
//! overlay back over the desktop. The class closed here is a fixup of
//! any kind for a forgotten window left due in either queue: waiting for
//! the window to be managed, or to be asserted again. Fixups for other
//! windows stay due. Not covered: what the dispatcher sends, and the
//! window manager's response, which `tests/park.rs` checks on Xvfb.

use std::time::Instant;

use super::{forget, Fixup, Pending, AGAIN, PATIENCE, PENDING};

/// The windows, among `of`, of each fixup that `PENDING` and `AGAIN`
/// hold.
fn due(of: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let waiting = PENDING
        .lock()
        .iter()
        .filter(|p| of.contains(&p.xid))
        .map(|p| p.xid)
        .collect();
    let again = AGAIN
        .lock()
        .iter()
        .filter(|a| of.contains(&a.1))
        .map(|a| a.1)
        .collect();
    (waiting, again)
}

#[test]
fn forgetting_a_window_drops_its_fixups_from_both_queues() {
    // Ids no X server issues in this process, kept apart from any other
    // case that touches the queues.
    let (parked, kept) = (0x7fff_0001, 0x7fff_0002);
    let span = Fixup::Span {
        x: 0,
        y: 0,
        w: 1,
        h: 1,
    };
    let now = Instant::now();
    for (xid, fixup) in [
        (parked, span),
        (parked, Fixup::AlwaysOnTop),
        (parked, Fixup::Unlisted),
        (kept, span),
    ] {
        PENDING.lock().push(Pending {
            xid,
            fixup,
            deadline: now + PATIENCE,
        });
        AGAIN.lock().push((now, xid, fixup));
    }
    let all = vec![parked, parked, parked, kept];
    assert_eq!(due(&[parked, kept]), (all.clone(), all));

    forget(parked);
    assert_eq!(due(&[parked, kept]), (vec![kept], vec![kept]));

    forget(kept);
    assert_eq!(due(&[parked, kept]), (vec![], vec![]));
}
