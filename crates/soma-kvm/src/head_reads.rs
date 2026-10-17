//! The bytes a guest read from its private head between resume and Ready.
//!
//! A head is a `FICLONE` of the Generation's overlay template, so it shares that template's
//! extents and costs no copy. What it does not share is the template's *page cache*: the page
//! cache belongs to the inode, and a fresh clone is a fresh inode with nothing resident. So the
//! first read of a head faults its pages in from the device, and on a loaded host that fault is
//! the whole of the launch's remaining latency.
//!
//! Which pages those are is a property of the Generation, not of the request: the guest is
//! deterministic, so every launch of one Generation reads the same few ranges at the same
//! offsets. Recording them once, on a launch that had to pay for them anyway, lets the pool warm
//! the next head for that Generation while it is being prepared, off the request path.
//!
//! A head is a disk, so the recorder is bounded twice over: a guest that reads its whole disk
//! would otherwise turn priming into a full read of it. Nothing recorded here is guest content -
//! only the offsets and lengths of reads, which is why the set is safe to carry off the host.

use std::sync::{Arc, Mutex, PoisonError};

/// Most bytes one Generation's warm set may hold.
///
/// The ranges a restore actually needs are a few pages; this is far above them and far below the
/// smallest head, so a pathological guest cannot make priming heavy.
pub const MAX_WARM_BYTES: u64 = 1024 * 1024;

/// Most ranges one Generation's warm set may hold.
pub const MAX_WARM_SPANS: usize = 32;

/// How much of a range is read at a time while warming.
const WARM_CHUNK: usize = 64 * 1024;

/// One range of a head a guest read, as offset and length in bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeadSpan {
    /// Byte offset from the start of the head.
    pub offset: u64,
    /// Length in bytes.
    pub len: u64,
}

/// A recorder and the ranges it has seen.
///
/// Reads are merged as they arrive, so a guest that faults a page at a time still yields one
/// span per region rather than one per page, and the bound is spent on distinct regions.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HeadReads {
    spans: Vec<HeadSpan>,
    total: u64,
    sealed: bool,
}

/// One recorder shared between the device that reads and the session that reports.
pub type HeadReadsHandle = Arc<Mutex<HeadReads>>;

/// A recorder nothing has read through yet.
#[must_use]
pub fn recording() -> HeadReadsHandle {
    Arc::new(Mutex::new(HeadReads::default()))
}

/// Runs `work` with the recorder behind `handle`, poisoning-proof.
///
/// A panic in a device thread must not turn every later read into a panic too; a recorder that
/// cannot be locked simply stops recording, which costs a warm set rather than a machine.
fn with_reads<T>(handle: &HeadReadsHandle, work: impl FnOnce(&mut HeadReads) -> T) -> T {
    let mut reads = handle.lock().unwrap_or_else(PoisonError::into_inner);
    work(&mut reads)
}

/// Records one read through the shared recorder, if it is still open and under the bound.
pub fn record(handle: &HeadReadsHandle, offset: u64, len: u64) {
    with_reads(handle, |reads| reads.record(offset, len));
}

/// Seals the shared recorder and takes the ranges it holds.
///
/// Sealing is what keeps the set to the window it is about: everything after Ready is a request
/// the guest chose to make, not a cost of the launch, and would pollute the warm set and spend
/// its bound.
pub fn seal(handle: &HeadReadsHandle) -> Vec<HeadSpan> {
    with_reads(handle, HeadReads::seal)
}

/// Reads every recorded range once, so its pages are resident before a guest asks for them.
///
/// Synchronous by design. A head is warmed before the host holding it counts as primed, so a
/// claim can never race a read that is still in flight.
///
/// # Errors
///
/// Fails when a range cannot be read. The caller decides whether an unwarmed head is fatal, and
/// it is not: the guest will fault the pages itself, which is what happens today.
#[cfg(unix)]
pub fn warm(file: &std::fs::File, spans: &[HeadSpan]) -> std::io::Result<u64> {
    use std::os::unix::fs::FileExt as _;

    let mut buffer = vec![0_u8; WARM_CHUNK];
    let mut warmed = 0_u64;
    for span in spans {
        let end = span.offset.saturating_add(span.len);
        let mut offset = span.offset;
        while offset < end {
            let want = usize::try_from((end - offset).min(WARM_CHUNK as u64)).unwrap_or(0);
            let Some(chunk) = buffer.get_mut(..want) else {
                break;
            };
            file.read_exact_at(chunk, offset)?;
            offset = offset.saturating_add(want as u64);
            warmed = warmed.saturating_add(want as u64);
        }
    }
    Ok(warmed)
}

impl HeadReads {
    /// Adds ranges observed by another host, under this recorder's own bounds.
    ///
    /// Used where a set recorded by one machine host is taken up by another. Going through
    /// [`Self::record`] rather than replacing the set is what keeps a later or larger observation
    /// from growing the set past [`MAX_WARM_BYTES`] or [`MAX_WARM_SPANS`].
    pub fn absorb(&mut self, spans: &[HeadSpan]) {
        for span in spans {
            self.record(span.offset, span.len);
        }
    }

    /// Records one read, merging it into the range it continues or overlaps.
    ///
    /// A read that would take the set past [`MAX_WARM_BYTES`] or [`MAX_WARM_SPANS`] is dropped
    /// whole rather than shortened: a partly warmed region is no warmer than an unwarmed one,
    /// and the first ranges a guest reads are the ones worth keeping.
    pub fn record(&mut self, offset: u64, len: u64) {
        if self.sealed || len == 0 {
            return;
        }
        let Some(end) = offset.checked_add(len) else {
            return;
        };
        // A read that continues the last range extends it; nothing else can be adjacent to it,
        // because a guest walks a region forward.
        if let Some(last) = self.spans.last_mut()
            && offset <= last.offset.saturating_add(last.len)
        {
            let extended = end.saturating_sub(last.offset);
            if extended <= last.len {
                return;
            }
            let grown = extended - last.len;
            if self.total.saturating_add(grown) > MAX_WARM_BYTES {
                return;
            }
            last.len = extended;
            self.total = self.total.saturating_add(grown);
            return;
        }
        if self.spans.len() >= MAX_WARM_SPANS || self.total.saturating_add(len) > MAX_WARM_BYTES {
            return;
        }
        self.spans.push(HeadSpan { offset, len });
        self.total = self.total.saturating_add(len);
    }

    /// Seals this recorder and returns what it holds.
    pub fn seal(&mut self) -> Vec<HeadSpan> {
        self.sealed = true;
        std::mem::take(&mut self.spans)
    }

    /// The ranges recorded so far, in the order the guest read them.
    #[must_use]
    pub fn spans(&self) -> &[HeadSpan] {
        &self.spans
    }

    /// The bytes the ranges hold together.
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total
    }

    /// Whether the window this recorder is about has closed.
    #[must_use]
    pub const fn is_sealed(&self) -> bool {
        self.sealed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walk_forward_through_one_region_is_a_single_span() {
        let mut reads = HeadReads::default();
        for page in 0..8 {
            reads.record(4096 * page, 4096);
        }
        assert_eq!(
            reads.spans(),
            &[HeadSpan {
                offset: 0,
                len: 4096 * 8
            }]
        );
        assert_eq!(reads.total_bytes(), 4096 * 8);
    }

    #[test]
    fn separate_regions_stay_separate() {
        let mut reads = HeadReads::default();
        reads.record(0x8000_0000, 4096);
        reads.record(0x8000_f000, 4096);
        assert_eq!(reads.spans().len(), 2);
        assert_eq!(reads.spans()[0].offset, 0x8000_0000);
        assert_eq!(reads.spans()[1].offset, 0x8000_f000);
    }

    #[test]
    fn a_reread_of_the_same_page_does_not_grow_the_set() {
        let mut reads = HeadReads::default();
        reads.record(0, 4096);
        reads.record(0, 4096);
        assert_eq!(reads.total_bytes(), 4096);
    }

    #[test]
    fn the_byte_bound_drops_a_range_rather_than_shortening_it() {
        let mut reads = HeadReads::default();
        reads.record(0, MAX_WARM_BYTES);
        let before = reads.clone();
        reads.record(MAX_WARM_BYTES * 4, 4096);
        assert_eq!(reads, before, "an over-bound read is refused whole");
        assert_eq!(reads.total_bytes(), MAX_WARM_BYTES);
    }

    #[test]
    fn the_span_bound_stops_the_set_growing() {
        let mut reads = HeadReads::default();
        for span in 0..MAX_WARM_SPANS {
            reads.record(span as u64 * 1_000_000, 512);
        }
        assert_eq!(reads.spans().len(), MAX_WARM_SPANS);
        let before = reads.clone();
        reads.record(1 << 40, 512);
        assert_eq!(reads, before);
    }

    #[test]
    fn sealing_closes_the_window_and_keeps_what_it_held() {
        let mut reads = HeadReads::default();
        reads.record(0, 4096);
        let taken = reads.seal();
        assert_eq!(taken.len(), 1);
        assert!(reads.is_sealed());
        reads.record(8192, 4096);
        assert_eq!(
            reads.spans().len(),
            0,
            "a read after Ready is a request, not a cost of the launch"
        );
    }

    #[test]
    fn a_shared_recorder_is_reachable_from_both_ends() {
        let handle = recording();
        record(&handle, 0, 4096);
        assert_eq!(
            seal(&handle),
            vec![HeadSpan {
                offset: 0,
                len: 4096
            }]
        );
        record(&handle, 8192, 4096);
        assert!(
            seal(&handle).is_empty(),
            "the second seal sees a closed set"
        );
    }

    #[test]
    fn a_zero_length_read_records_nothing() {
        let mut reads = HeadReads::default();
        reads.record(0, 0);
        assert!(reads.spans().is_empty());
    }

    #[test]
    fn an_offset_that_would_wrap_records_nothing() {
        let mut reads = HeadReads::default();
        reads.record(u64::MAX, 4096);
        assert!(reads.spans().is_empty());
    }
}
