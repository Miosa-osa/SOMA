//! Public result of asking the owning host to capture one running Instance.

use serde::{Deserialize, Serialize};
use soma::{GenerationId, InstanceId, OperationId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointReceipt {
    pub instance_id: InstanceId,
    pub checkpoint_id: OperationId,
    pub generation_id: GenerationId,
    pub state_digest: String,
    pub memory_digest: String,
    pub memory_bytes: u64,
    pub overlay_bytes: u64,
    pub overlay_digest: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointFailure {
    NotFound,
    Unsupported,
    CaptureRefused,
    Storage,
    Unconfirmed,
}
