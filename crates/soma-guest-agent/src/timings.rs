//! Fixed-slot timing of the repair path, reported once on the console after readiness.
//!
//! The host timeline can only see the four intervals between resume and `Ready`; this module
//! is the guest half of that measurement, so each interval can be attributed to waiting or to
//! work without guessing.
//! One step costs two monotonic reads and one relaxed store, the slots are a fixed array that
//! never allocates or grows, and nothing is rendered at all unless the `timing-report` feature
//! is on, which the shipped agent does not build with: the rendering is investigation
//! scaffolding, and an Instance should not spend its console on it between readiness and its
//! first request. Rebuild with `SOMA_GUEST_AGENT_FEATURES=timing-report
//! ./scripts/build-guest-agent.sh` to read the steps again.
//! Every value is a duration in microseconds and carries no identity, key, or peer byte.

#[cfg(any(test, feature = "timing-report"))]
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// One measured step, in the order the agent reaches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Step {
    /// Observed length of the launch-page poll sleep the restore interrupted.
    PollWake,
    /// The domain probe of the mapped page that found the fresh material.
    PageLook,
    /// Copying the page out of the `/dev/mem` view into locked memory.
    PageCopy,
    /// Overwriting the view with zeroes and reading every byte back.
    PageErase,
    /// Validating and parsing the locked copy.
    PageParse,
    /// Reading one contribution from the virtio entropy device.
    EntropyRead,
    /// Mixing both contributions and forcing the CRNG reseed.
    EntropyMix,
    /// Proving `getrandom` no longer blocks.
    EntropyVerify,
    /// Waiting for the vsock device to report the assigned context identifier.
    CidWait,
    /// Creating and connecting the vsock control socket.
    VsockConnect,
    /// Identity repair.
    Identity,
    /// Network repair.
    Network,
    /// Handshake time blocked reading the host's first message.
    HandshakeWait,
    /// Handshake time writing the second message.
    HandshakeSend,
    /// Handshake time that is not transport, which is the Noise work.
    HandshakeWork,
    /// Time blocked waiting for the `PrepareAndProbe` request.
    RequestWait,
    /// Sending the `RepairComplete` report.
    RepairReport,
    /// Forking and executing the probe.
    Spawn,
    /// Streaming the probe's pipes until both close.
    Stream,
    /// Waiting for the probe's exit status after its pipes closed.
    Wait,
    /// Killing, reaping, and sweeping every descendant.
    Reap,
    /// Sending the terminal report.
    TerminalReport,
}

/// Slots, one per [`Step`].
const STEPS: usize = 22;
/// Steps rendered on one console line; two lines cover every slot.
#[cfg(any(test, feature = "timing-report"))]
const PER_LINE: usize = STEPS / 2;

/// Short console label of each slot, in [`Step`] order.
#[cfg(any(test, feature = "timing-report"))]
const LABELS: [&str; STEPS] = [
    "wake", "look", "copy", "erase", "parse", "hwrng", "mix", "crng", "cid", "vsock", "ident",
    "net", "hswait", "hssend", "hswork", "req", "report", "spawn", "stream", "wait", "reap",
    "term",
];

static ELAPSED: [AtomicU64; STEPS] = [const { AtomicU64::new(0) }; STEPS];
static READ_NANOS: AtomicU64 = AtomicU64::new(0);
static WRITE_NANOS: AtomicU64 = AtomicU64::new(0);
/// Whether the transport totals are still wanted; [`around`] clears it once it has read them.
static ARMED: AtomicBool = AtomicBool::new(true);

/// Records how long one step took, replacing any earlier value for it.
pub fn record(step: Step, elapsed: Duration) {
    ELAPSED[step as usize].store(nanos(elapsed), Ordering::Relaxed);
}

/// Runs `work`, records how long it took as `step`, and returns its value.
pub fn measure<T>(step: Step, work: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let value = work();
    record(step, started.elapsed());
    value
}

/// Runs `work` and splits its duration into transport reads, transport writes, and the rest.
///
/// The three slots together account for the whole call, so a stage that turns out to be
/// blocked on the peer cannot be mistaken for one that is computing.
pub fn around<T>(wait: Step, send: Step, rest: Step, work: impl FnOnce() -> T) -> T {
    let (read_before, write_before) = transport();
    let started = Instant::now();
    let value = work();
    let (read_after, write_after) = transport();
    let waited = read_after.saturating_sub(read_before);
    let written = write_after.saturating_sub(write_before);
    ELAPSED[wait as usize].store(waited, Ordering::Relaxed);
    ELAPSED[send as usize].store(written, Ordering::Relaxed);
    ELAPSED[rest as usize].store(
        nanos(started.elapsed()).saturating_sub(waited.saturating_add(written)),
        Ordering::Relaxed,
    );
    // The totals have been consumed; every later transport call runs without a clock read.
    ARMED.store(false, Ordering::Relaxed);
    value
}

/// Runs one control-transport read, timing it while the split above still needs the total.
pub fn transport_read<T>(work: impl FnOnce() -> T) -> T {
    timed(&READ_NANOS, work)
}

/// Runs one control-transport write, timing it while the split above still needs the total.
pub fn transport_write<T>(work: impl FnOnce() -> T) -> T {
    timed(&WRITE_NANOS, work)
}

/// Adds one call's duration to `total`, or runs it untimed once the totals are spent.
///
/// The handshake is the last thing that reads them, and it happens before the first tenant
/// command, so after it every output chunk would pay two clock reads for a number nothing
/// consumes.
fn timed<T>(total: &AtomicU64, work: impl FnOnce() -> T) -> T {
    if !ARMED.load(Ordering::Relaxed) {
        return work();
    }
    let started = Instant::now();
    let value = work();
    total.fetch_add(nanos(started.elapsed()), Ordering::Relaxed);
    value
}

/// Renders every slot as two bounded console lines of microsecond values.
#[cfg(any(test, feature = "timing-report"))]
#[must_use]
pub fn lines() -> [String; 2] {
    let mut lines = [String::new(), String::new()];
    for (index, line) in lines.iter_mut().enumerate() {
        let _ = write!(line, "timing {}", index + 1);
        for slot in index * PER_LINE..(index + 1) * PER_LINE {
            let micros = ELAPSED[slot].load(Ordering::Relaxed) / 1_000;
            let _ = write!(line, " {}={micros}", LABELS[slot]);
        }
    }
    lines
}

fn transport() -> (u64, u64) {
    (
        READ_NANOS.load(Ordering::Relaxed),
        WRITE_NANOS.load(Ordering::Relaxed),
    )
}

fn nanos(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console;
    use std::thread;

    /// Nine seconds in nanoseconds: the widest value any real step can plausibly render.
    const WIDE: u64 = 9_000_000_000;

    #[test]
    fn every_step_has_exactly_one_slot_in_declaration_order() {
        assert_eq!(Step::PollWake as usize, 0);
        assert_eq!(Step::HandshakeWait as usize, PER_LINE + 1);
        assert_eq!(Step::TerminalReport as usize, STEPS - 1);
        assert_eq!(LABELS.len(), ELAPSED.len());
    }

    #[test]
    fn the_widest_report_still_fits_one_console_line_per_half() {
        for slot in &ELAPSED {
            slot.store(WIDE, Ordering::Relaxed);
        }

        for line in lines() {
            assert!(line.starts_with("timing "));
            assert_eq!(line.matches('=').count(), PER_LINE);
            // A shortened line would lose values, so the console must keep every byte.
            assert_eq!(console::bounded_line(&line).len(), line.len() + 19);
        }
    }

    /// Both halves are one test because `around` disarms the transport clock for the process.
    #[test]
    fn a_split_call_attributes_transport_time_away_from_work_and_then_stops_the_clock() {
        around(
            Step::HandshakeWait,
            Step::HandshakeSend,
            Step::HandshakeWork,
            || {
                transport_read(|| thread::sleep(Duration::from_millis(4)));
                transport_write(|| thread::sleep(Duration::from_millis(1)));
            },
        );

        // Other tests share the process-wide totals, so the split can only be larger.
        assert!(ELAPSED[Step::HandshakeWait as usize].load(Ordering::Relaxed) >= 4_000_000);
        assert!(ELAPSED[Step::HandshakeSend as usize].load(Ordering::Relaxed) >= 1_000_000);
        assert!(ELAPSED[Step::HandshakeWork as usize].load(Ordering::Relaxed) < 1_000_000);

        // Every tenant command's output crosses the same transport; nothing reads the totals
        // again, so nothing may keep adding to them.
        let spent = transport();
        transport_read(|| thread::sleep(Duration::from_millis(2)));
        transport_write(|| thread::sleep(Duration::from_millis(2)));

        assert_eq!(
            transport(),
            spent,
            "the transport clock kept running after the handshake consumed it"
        );
    }
}
