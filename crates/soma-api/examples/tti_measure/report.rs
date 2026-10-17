//! What the measurement prints: one line per flow, and the arithmetic behind it.
//!
//! Every figure is a whole number of microseconds until it is printed, so nothing here rounds a
//! sample on the way in and a reader can check the table against the transcript.

use std::time::Instant;

use soma_api::runner::service::RunnerResponse;

/// The segments of one answer, in microseconds, and the body's own create time.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timing {
    pub(super) pool_us: u64,
    pub(super) exec_us: u64,
    pub(super) ready_us: u64,
    pub(super) wait_us: Option<u64>,
    /// The `run` segment: the part of `exec` a create-and-run's command spent.
    pub(super) run_us: Option<u64>,
    /// The body's `create_ms`, which is the `pool` and `exec` the same answer reports.
    pub(super) create_ms: u64,
}

impl Timing {
    pub(super) fn of(answer: &RunnerResponse) -> Self {
        let header = answer
            .headers
            .iter()
            .find(|(name, _)| *name == "server-timing")
            .map(|(_, value)| value.as_str())
            .expect("every runner answer carries a server-timing header");
        let named = |wanted: &str| -> Option<u64> {
            for entry in header.split(',') {
                let Some((name, value)) = entry.split_once(";dur=") else {
                    continue;
                };
                if name != wanted {
                    continue;
                }
                let (whole, fraction) = value.split_once('.')?;
                let whole: u64 = whole.parse().ok()?;
                let fraction: u64 = fraction.parse().ok()?;
                return Some(whole * 1_000 + fraction);
            }
            None
        };
        Self {
            pool_us: named("pool").unwrap_or(0),
            exec_us: named("exec").unwrap_or(0),
            ready_us: named("ready").unwrap_or(0),
            wait_us: named("wait"),
            run_us: named("run"),
            create_ms: answer_body(answer)["create_ms"].as_u64().unwrap_or(0),
        }
    }
}

/// One answer's body, decoded.
pub(super) fn answer_body(answer: &RunnerResponse) -> serde_json::Value {
    serde_json::from_slice(&answer.body).unwrap_or(serde_json::Value::Null)
}

/// Whole microseconds since `started`, without a float anywhere near a sample.
pub(super) fn elapsed_us(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Milliseconds, with the microseconds the header carries.
pub(super) fn millis(micros: u64) -> String {
    format!("{}.{:03}", micros / 1_000, micros % 1_000)
}

/// The median of one set of samples, in microseconds.
pub(super) fn median(samples: &mut [u64]) -> u64 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

/// The 90th percentile of one set of samples, in microseconds.
fn ninetieth(samples: &mut [u64]) -> u64 {
    samples.sort_unstable();
    samples[(samples.len() * 9) / 10]
}

/// One line of the table: what one flow cost, as the caller saw it and as the runner reported it.
pub(super) fn report(label: &str, totals: &mut [u64], detail: &str) {
    println!(
        "  {label:<22} n={:<4} total p50={:>10} ms  p90={:>10} ms  {detail}",
        totals.len(),
        millis(median(totals)),
        millis(ninetieth(totals)),
    );
}

/// The median of samples that a flow may not have produced at all, as a printed figure.
pub(super) fn optional(samples: &mut [u64]) -> String {
    if samples.is_empty() {
        "none".to_owned()
    } else {
        format!("{} ms", millis(median(samples)))
    }
}
