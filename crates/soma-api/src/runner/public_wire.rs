//! The public response bodies, byte-compatible with the fast lane they replace.
//!
//! Contract C2 keeps every body the fast-lane listener (`Web.SomaFastLaneEdge.Listener` and
//! `Web.SomaFastLane.Responder`, miosa-compute 71c9430633) returns. Those are Elixir maps
//! rendered by Jason, and a small Elixir map renders its keys in sorted order, so every struct
//! here declares its fields alphabetically; `serde_json` then writes the same bytes. The
//! runner's own refusals (401, 403, 421, 429, 503) are the short bodies C2 defines for them.

use serde::Serialize;

pub use crate::runner::refusals::{health, misdirected, refusal};

mod error;
mod run;

pub use error::PlatformError;
pub use run::Ran;

/// The compact `201` body `SandboxView.render_compact/1` produces.
#[derive(Serialize)]
pub struct Created<'a> {
    pub cpu_count: u16,
    /// Server-side create time in whole milliseconds: the sum of the `pool` and `exec` segments
    /// the same answer reports in `Server-Timing` (plan track T4, an added key).
    pub create_ms: u64,
    pub created_at: &'a str,
    pub deletion_pending: bool,
    pub id: &'a str,
    pub memory_mb: u64,
    pub name: Option<&'a str>,
    /// The network this sandbox was created with, echoed only when the create named one, so a
    /// caller can tell a runner that honoured `network.egress` from an older one that ignored it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkEcho<'a>>,
    /// The command a create-and-run ran, and what it produced.
    ///
    /// Only a create that carried a `run` block has one, and the key is left out entirely for
    /// every other create, so a body that a client read before this field existed is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run: Option<&'a Ran<'a>>,
    /// Where every later call for this sandbox goes (contract C2). An extra key, which the
    /// compact body's JSON clients ignore; it sorts where Jason would put it.
    pub runner_url: &'a str,
    pub slug: &'a str,
    pub state: &'a str,
    pub template_id: &'a str,
    pub timeout_sec: u64,
}

/// The `network` object a create that named one echoes.
#[derive(Serialize)]
pub struct NetworkEcho<'a> {
    /// `denied`, `internet` or `unrestricted`.
    pub egress: &'a str,
}

/// The `200` exec body: `%{data: %{stdout, stderr, exit_code}}`.
#[derive(Serialize)]
pub struct Executed<'a> {
    pub data: ExecutedData<'a>,
}

#[derive(Serialize)]
pub struct ExecutedData<'a> {
    pub exit_code: i32,
    pub stderr: &'a str,
    pub stdout: &'a str,
}

/// The `200` destroy body `Responder.destroy_result/2` produces.
#[derive(Serialize)]
pub struct Destroyed<'a> {
    /// Guest CPU time; `null` until soma-api exposes per-sandbox CPU accounting (C7).
    pub cpu_ms: Option<u64>,
    pub id: &'a str,
    /// Wall time from create to destroy (C7). This is the sandbox's uptime: the clock starts
    /// when the create claimed it and stops when the destroy answered, so it is reported here
    /// once rather than duplicated into a second `uptime_ms` key.
    pub lifetime_ms: u64,
    /// Peak resident bytes the sandbox used, `null` until the machine host exposes per-sandbox
    /// peak memory: the facade's receipt and the destroy answer carry no such figure today, and
    /// a stand-in derived from the shape would be the shape's ceiling, not a measurement.
    pub mem_peak_bytes: Option<u64>,
    pub operation_id: Option<&'a str>,
    pub state: &'a str,
    pub total_runtime_sec: Option<u64>,
}

/// The create-capacity body `CreateErrors.capacity_error/5` produces.
#[derive(Serialize)]
struct CapacityBody<'a> {
    error: CapacityFields<'a>,
}

#[derive(Serialize)]
struct CapacityFields<'a> {
    code: &'a str,
    message: &'a str,
    retry_after: u64,
    retryable: bool,
}

/// `Responder.unavailable/2`: the lane could not claim a prepared sandbox.
#[must_use]
pub fn create_unavailable() -> Vec<u8> {
    encode(&CapacityBody {
        error: CapacityFields {
            code: "SOMA_FAST_LANE_UNAVAILABLE",
            message: "The SOMA fast lane could not claim a prepared sandbox right now. Retry in a moment.",
            retry_after: 1,
            retryable: true,
        },
    })
}

#[must_use]
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    // Every body here is built from strings, numbers, booleans, and nulls, which always encode.
    serde_json::to_vec(value).unwrap_or_else(|_| br#"{"error":"internal"}"#.to_vec())
}

#[cfg(test)]
#[path = "public_wire_tests.rs"]
mod tests;
