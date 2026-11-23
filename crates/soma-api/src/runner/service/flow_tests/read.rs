//! Read a runner answer back: the created id, its timings and body, and wait for the reaper.

use super::{Duration, Engine, Instant, Ordering, Runner, RunnerResponse};

pub(in super::super) fn created_id(response: &RunnerResponse) -> String {
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("JSON");
    body["id"].as_str().expect("id").to_owned()
}

/// The `Server-Timing` segments one answer reports, in microseconds.
///
/// The header carries milliseconds with three decimals, so every value here is exact rather than
/// rounded: a test can compare a segment with the body's own `create_ms` instead of with a
/// tolerance.
pub(in super::super) fn timing_of(
    response: &RunnerResponse,
) -> std::collections::BTreeMap<String, u64> {
    let header = response
        .headers
        .iter()
        .find(|(name, _)| *name == "server-timing")
        .map(|(_, value)| value.as_str())
        .expect("every answer carries a server-timing header");
    // A metric with a description and no duration, such as `launch;desc="prepared"`, is a fact
    // about the request rather than a span of it, so it has no place in a map of durations.
    header
        .split(',')
        .filter_map(|segment| {
            let (name, value) = segment.split_once(";dur=")?;
            let (millis, micros) = value.split_once('.').expect("three decimals");
            Some((
                name.to_owned(),
                millis.parse::<u64>().expect("millis") * 1_000
                    + micros.parse::<u64>().expect("micros"),
            ))
        })
        .collect()
}

/// The decoded body of one answer.
pub(in super::super) fn body_of(response: &RunnerResponse) -> serde_json::Value {
    serde_json::from_slice(&response.body).expect("the answer is JSON")
}

/// Sweeps until the expired sandbox is gone, or fails once the deadline passes.
///
/// A sweep compares wall-clock instants, so waiting on the condition rather than on a fixed margin
/// is what keeps these tests about expiry and not about how busy the machine running them is.
pub(in super::super) async fn reap_until_destroyed(runner: &Runner, engine: &Engine) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine.destroys.load(Ordering::SeqCst) == 0 {
        assert!(Instant::now() < deadline, "the reaper never destroyed it");
        tokio::time::sleep(Duration::from_millis(10)).await;
        runner.reap().await;
    }
}
