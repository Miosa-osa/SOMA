//! What the envelope says about the set of sandboxes this state root holds.
//!
//! Two facts per sandbox, kept apart. `state` is what the durable record says its last completed
//! transition left it in; `host` is what the backend could still reach when the listing ran. A
//! record can say `active` while the process that held its machine is gone, so collapsing the two
//! into one word would report a dead sandbox as a usable one.

use serde::Serialize;
use soma::{BackendKind, InstanceId, SandboxEntry};

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct SandboxListEntry {
    pub instance_id: InstanceId,
    pub state: &'static str,
    pub host: &'static str,
    pub backend: BackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct SandboxListReport {
    pub sandboxes: Vec<SandboxListEntry>,
    pub count: usize,
}

impl SandboxListReport {
    /// Builds the report from what the engine enumerated.
    #[must_use]
    pub fn new(entries: &[SandboxEntry]) -> Self {
        let sandboxes: Vec<SandboxListEntry> = entries
            .iter()
            .map(|entry| SandboxListEntry {
                instance_id: entry.instance_id().clone(),
                state: entry.phase().code(),
                host: entry.liveness().code(),
                backend: entry.backend(),
                name: entry.name().map(|name| name.as_str().to_owned()),
            })
            .collect();
        Self {
            count: sandboxes.len(),
            sandboxes,
        }
    }
}
