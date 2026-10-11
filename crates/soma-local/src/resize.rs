//! Changing a running sandbox's CPU and memory, and saying exactly what changed.
//!
//! Every dimension answers with its own state. A caller that asks for more memory than the guest
//! took does not get the amount it asked for: it gets what the device confirmed, so the number a
//! customer is shown is the number the guest's `MemTotal` shows.

use serde::{Deserialize, Serialize};

/// What a caller wants changed. A dimension left out is left alone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResizeRequest {
    /// The vCPUs the machine may use, between one and the count it booted with.
    pub cpu_count: Option<u16>,
    /// The total memory the machine should have, between what it booted with and its ceiling.
    pub memory_mb: Option<u64>,
}

/// How one dimension ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionState {
    /// The change is in effect and was confirmed.
    AppliedLive,
    /// Part of the change is in effect; the reported value is what actually is.
    Partial,
    /// Nothing was asked, or the machine already had it.
    Unchanged,
    /// This machine cannot do it live; `reason` says why.
    Unsupported,
    /// It was attempted and failed; the reported value is what is still true.
    Failed,
}

/// The CPU dimension of an answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CpuResize {
    /// How the dimension ended.
    pub state: DimensionState,
    /// The vCPUs asked for, when anything was.
    pub requested: Option<u16>,
    /// The vCPUs the machine is now limited to.
    pub applied: u16,
    /// The vCPUs the machine booted with, which is the most it can have.
    pub booted: u16,
    /// Why nothing, or less than asked, was done.
    pub reason: Option<String>,
}

/// The memory dimension of an answer.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryResize {
    /// How the dimension ended.
    pub state: DimensionState,
    /// The total asked for, when anything was.
    pub requested_mb: Option<u64>,
    /// The memory plugged on top of what the machine booted with, as the device confirmed it.
    pub plugged_mb: u64,
    /// What the guest now has: boot plus plugged.
    pub total_mb: u64,
    /// What the machine booted with.
    pub booted_mb: u64,
    /// The most the machine can have.
    pub ceiling_mb: u64,
    /// Why nothing, or less than asked, was done.
    pub reason: Option<String>,
}

/// Both dimensions of one resize.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResizeOutcome {
    /// The process that holds the machine, so an operator can find its memory and CPU.
    #[serde(default)]
    pub host_pid: u32,
    /// The CPU dimension.
    pub cpu: CpuResize,
    /// The memory dimension.
    pub memory: MemoryResize,
}

impl ResizeOutcome {
    /// Whether anything changed live.
    #[must_use]
    pub fn applied_live(&self) -> bool {
        let live = |state| matches!(state, DimensionState::AppliedLive | DimensionState::Partial);
        live(self.cpu.state) || live(self.memory.state)
    }
}

/// Why a resize could not be attempted at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeFailure {
    /// No hosted sandbox with this identity is reachable on this host.
    NotFound,
    /// The exchange failed.
    Unavailable,
}

/// Asks the process hosting a sandbox to resize it live.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) fn resize_machine(
    directory: &std::path::Path,
    instance: &soma::InstanceId,
    request: ResizeRequest,
) -> Result<ResizeOutcome, ResizeFailure> {
    crate::backend::resize_machine(directory, instance, request)
}

/// Only the KVM backend resizes live.
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn resize_machine(
    _directory: &std::path::Path,
    _instance: &soma::InstanceId,
    _request: ResizeRequest,
) -> Result<ResizeOutcome, ResizeFailure> {
    Err(ResizeFailure::Unavailable)
}
