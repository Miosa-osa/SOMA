//! Requests to pause one hosted sandbox.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseFailure {
    NotFound,
    Unavailable,
}

pub type ResumeFailure = PauseFailure;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) use crate::backend::{pause_machine, resume_machine};

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn pause_machine(
    _directory: &std::path::Path,
    _instance: &soma::InstanceId,
) -> Result<(), PauseFailure> {
    Err(PauseFailure::Unavailable)
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn resume_machine(
    _directory: &std::path::Path,
    _instance: &soma::InstanceId,
) -> Result<(), ResumeFailure> {
    Err(ResumeFailure::Unavailable)
}
