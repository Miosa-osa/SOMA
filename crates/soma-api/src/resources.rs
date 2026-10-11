//! `GET` and `PATCH /v1/sandboxes/{id}/resources`: what a running sandbox has, and changing it live.
//!
//! A change reports each dimension on its own, because a caller that asks for more memory than
//! the guest took must see what the device confirmed, not what it asked for. `applied_live` says
//! whether anything is now in effect. A `PATCH` that applies nothing, because the Generation
//! cannot do it or the attempt failed, is a `409` that still carries the same report, so the
//! caller reads the reason from the same place it reads a success.

use serde::{Deserialize, Serialize};
use soma_local::{CpuResize, DimensionState, MemoryResize, ResizeOutcome, ResizeRequest};

use crate::envelope::ApiError;

/// How CPU is limited: a CFS quota on the machine's cgroup, so the guest still sees its booted
/// vCPUs.
pub const CPU_MECHANISM: &str = "cgroup_quota";
/// How memory is changed: the virtio-mem device plugs and unplugs blocks.
pub const MEMORY_MECHANISM: &str = "virtio_mem";

/// What a `PATCH` asks for; a field left out is left alone.
///
/// `operation_id` names the attempt (32 lowercase hex, a compact UUID). The same id sent again
/// for the same sandbox is answered with the stored answer instead of being applied twice, which
/// is how a caller whose delivery was ambiguous learns what happened.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcesBody {
    pub cpu_count: Option<u16>,
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub operation_id: Option<String>,
}

impl ResourcesBody {
    /// The operation id, when the caller named one and it is well formed.
    ///
    /// # Errors
    ///
    /// Returns a `400` for an id that is not 32 lowercase hex characters.
    pub fn operation(&self) -> Result<Option<&str>, ApiError> {
        match self.operation_id.as_deref() {
            None => Ok(None),
            Some(id)
                if id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) =>
            {
                Ok(Some(id))
            }
            Some(_) => Err(ApiError::new(
                400,
                "invalid_resize",
                "operation_id must be 32 lowercase hex characters",
                false,
            )),
        }
    }

    /// The facade request this body means.
    ///
    /// # Errors
    ///
    /// Returns a `400` for a body that asks for nothing or for zero of something.
    pub fn into_request(&self) -> Result<ResizeRequest, ApiError> {
        if self.cpu_count.is_none() && self.memory_mb.is_none() {
            return Err(ApiError::new(
                400,
                "invalid_resize",
                "name cpu_count, memory_mb, or both",
                false,
            ));
        }
        if self.cpu_count == Some(0) || self.memory_mb == Some(0) {
            return Err(ApiError::new(
                400,
                "invalid_resize",
                "cpu_count and memory_mb must be positive",
                false,
            ));
        }
        Ok(ResizeRequest {
            cpu_count: self.cpu_count,
            memory_mb: self.memory_mb,
        })
    }
}

#[derive(Serialize)]
pub struct ResourcesReport<'a> {
    pub instance_id: &'a soma::InstanceId,
    /// Echoes the attempt's id when the caller named one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<&'a str>,
    pub applied_live: bool,
    pub cpu: CpuReport<'a>,
    pub memory: MemoryReport<'a>,
}

#[derive(Serialize)]
pub struct CpuReport<'a> {
    pub state: DimensionState,
    pub requested: Option<u16>,
    pub applied: u16,
    pub booted: u16,
    pub mechanism: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'a str>,
}

#[derive(Serialize)]
pub struct MemoryReport<'a> {
    pub state: DimensionState,
    pub requested_mb: Option<u64>,
    pub plugged_mb: u64,
    pub total_mb: u64,
    pub booted_mb: u64,
    pub ceiling_mb: u64,
    pub mechanism: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'a str>,
}

impl<'a> ResourcesReport<'a> {
    #[must_use]
    pub fn new(
        instance_id: &'a soma::InstanceId,
        operation_id: Option<&'a str>,
        outcome: &'a ResizeOutcome,
    ) -> Self {
        let CpuResize {
            state,
            requested,
            applied,
            booted,
            reason,
        } = &outcome.cpu;
        let MemoryResize {
            state: memory_state,
            requested_mb,
            plugged_mb,
            total_mb,
            booted_mb,
            ceiling_mb,
            reason: memory_reason,
        } = &outcome.memory;
        Self {
            instance_id,
            operation_id,
            applied_live: outcome.applied_live(),
            cpu: CpuReport {
                state: *state,
                requested: *requested,
                applied: *applied,
                booted: *booted,
                mechanism: CPU_MECHANISM,
                reason: reason.as_deref(),
            },
            memory: MemoryReport {
                state: *memory_state,
                requested_mb: *requested_mb,
                plugged_mb: *plugged_mb,
                total_mb: *total_mb,
                booted_mb: *booted_mb,
                ceiling_mb: *ceiling_mb,
                mechanism: MEMORY_MECHANISM,
                reason: memory_reason.as_deref(),
            },
        }
    }
}

/// Whether a change that was asked for ended with nothing in effect and something refused.
///
/// A request the machine already satisfied (`Unchanged` on both) is not a failure.
#[must_use]
pub fn nothing_applied(outcome: &ResizeOutcome) -> bool {
    let refused = |state| matches!(state, DimensionState::Unsupported | DimensionState::Failed);
    !outcome.applied_live() && (refused(outcome.cpu.state) || refused(outcome.memory.state))
}

/// Sandboxes whose last answered operation is remembered.
const REPLAY_CAPACITY: usize = 8_192;

/// The last answered operation per sandbox: its id, status and body.
type Remembered = std::collections::HashMap<String, (String, u16, Vec<u8>)>;

fn replays() -> &'static std::sync::Mutex<Remembered> {
    static REPLAYS: std::sync::OnceLock<std::sync::Mutex<Remembered>> = std::sync::OnceLock::new();
    REPLAYS.get_or_init(|| std::sync::Mutex::new(Remembered::new()))
}

/// The stored answer when `operation_id` is the last one answered for `instance`.
#[must_use]
pub fn replayed(instance: &soma::InstanceId, operation_id: &str) -> Option<(u16, Vec<u8>)> {
    let table = replays()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    table
        .get(instance.as_str())
        .filter(|(id, _, _)| id == operation_id)
        .map(|(_, status, body)| (*status, body.clone()))
}

/// Remembers the answer to `operation_id` for `instance`, replacing its previous one.
///
/// Only the latest attempt per sandbox is kept: a caller replays the attempt it is unsure about,
/// not an old one. When the table is full it is emptied rather than grown, which at worst makes a
/// replay apply again, and an apply to the same target is `unchanged`.
pub fn remember(instance: &soma::InstanceId, operation_id: &str, status: u16, body: &[u8]) {
    let mut table = replays()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if table.len() >= REPLAY_CAPACITY && !table.contains_key(instance.as_str()) {
        table.clear();
    }
    table.insert(
        instance.as_str().to_owned(),
        (operation_id.to_owned(), status, body.to_vec()),
    );
}
