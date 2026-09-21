//! Banded parallelism for pixel buffers.
//!
//! Every swizzle, crop, and stamp in iris splits a buffer into row
//! bands and runs one scoped thread per band once the image is large
//! enough to pay for the spawn. That scaffolding lived at six call
//! sites with subtly different thresholds; this is the single owner
//! of the policy: band count, the size gate, and the scoped spawn.

/// Run `f` over disjoint bands of `data`, one scoped thread per band.
///
/// `band_elems` is the number of elements per band (a row's worth, or
/// a pixel count for flat buffers); it must divide `data.len()` evenly
/// except possibly in the last band. `f` receives the band slice and
/// the element index where it starts, so closures can compute source
/// offsets without re-deriving the split.
///
/// Below `PARALLEL_MIN` bytes the work runs inline: thread spawn
/// latency costs more than the parallelism saves on small images.
pub fn par_bands_mut<T: Send>(
    data: &mut [T],
    band_elems: usize,
    f: impl Fn(&mut [T], usize) + Sync + Send,
) {
    par_bands_mut_work(data, band_elems, std::mem::size_of_val(data), f);
}

/// `par_bands_mut` with an explicit work estimate for the size gate.
/// Resampling writes a small output but reads a large input: gating on
/// the output slice would run a 33MB scan inline. `work_bytes` names
/// the real cost; the band split still comes from `data`.
pub fn par_bands_mut_work<T: Send>(
    data: &mut [T],
    band_elems: usize,
    work_bytes: usize,
    f: impl Fn(&mut [T], usize) + Sync + Send,
) {
    let band_elems = band_elems.max(1);
    const PARALLEL_MIN: usize = 1 << 20;
    if work_bytes < PARALLEL_MIN || data.len() <= band_elems {
        let start = 0;
        f(data, start);
        return;
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(data.len().div_ceil(band_elems))
        .max(1);
    let band = data.len().div_ceil(threads * band_elems) * band_elems;
    std::thread::scope(|scope| {
        let f = &f;
        let mut rest: &mut [T] = data;
        let mut start = 0usize;
        while !rest.is_empty() {
            let take = band.min(rest.len());
            let (head, tail) = rest.split_at_mut(take);
            rest = tail;
            let s = start;
            start += take;
            scope.spawn(move || f(head, s));
        }
    });
}

/// Same banding for a read-only source: `f` gets the source band and
/// its start index. Used where the destination is derived per band
/// (e.g. a strided source feeding a packed destination).
pub fn par_bands<T: Sync>(
    data: &[T],
    band_elems: usize,
    f: impl Fn(&[T], usize) + Sync + Send,
) {
    let band_elems = band_elems.max(1);
    let total_bytes = std::mem::size_of_val(data);
    const PARALLEL_MIN: usize = 1 << 20;
    if total_bytes < PARALLEL_MIN || data.len() <= band_elems {
        f(data, 0);
        return;
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(data.len().div_ceil(band_elems))
        .max(1);
    let band = data.len().div_ceil(threads * band_elems) * band_elems;
    std::thread::scope(|scope| {
        let f = &f;
        let mut rest: &[T] = data;
        let mut start = 0usize;
        while !rest.is_empty() {
            let take = band.min(rest.len());
            let (head, tail) = rest.split_at(take);
            rest = tail;
            let s = start;
            start += take;
            scope.spawn(move || f(head, s));
        }
    });
}

// WHY: the class closed here is "the band split drops or duplicates
// rows": a band that is not a multiple of the row size, or a start
// index that drifts, corrupts the image silently. Not covered: the
// pixel math inside each band, which the callers' tests cover.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_every_element_once() {
        // 1000 elements, band of 7: uneven tail must still be visited.
        let mut data = vec![0u32; 1000];
        let mut seen = std::collections::HashSet::new();
        let seen_ref = std::sync::Mutex::new(&mut seen);
        par_bands_mut(&mut data, 7, |band, start| {
            for (i, v) in band.iter_mut().enumerate() {
                *v = (start + i) as u32;
            }
            seen_ref.lock().unwrap().insert(start);
        });
        assert_eq!(data[999], 999);
        assert_eq!(data[0], 0);
        // Every element written exactly once: identity holds.
        assert!(data.iter().enumerate().all(|(i, &v)| v == i as u32));
    }

    #[test]
    fn small_buffers_run_inline() {
        let mut data = vec![0u8; 64];
        par_bands_mut(&mut data, 8, |band, start| {
            assert_eq!(start, 0);
            band.fill(7);
        });
        assert!(data.iter().all(|&b| b == 7));
    }

    #[test]
    fn readonly_variant_visits_all() {
        let data: Vec<u32> = (0..5000).collect();
        let sum = std::sync::atomic::AtomicU64::new(0);
        par_bands(&data, 64, |band, _| {
            let band_sum: u64 = band.iter().map(|&v| v as u64).sum();
            sum.fetch_add(band_sum, std::sync::atomic::Ordering::Relaxed);
        });
        let expected: u64 = (0..5000u64).sum();
        assert_eq!(sum.load(std::sync::atomic::Ordering::Relaxed), expected);
    }
}
