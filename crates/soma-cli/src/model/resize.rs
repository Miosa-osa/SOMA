//! What the envelope says about one live resize.

use serde::Serialize;
use soma::InstanceId;
use soma_local::{CpuResize, MemoryResize, ResizeOutcome};

/// Both dimensions of one resize, each with its own state and the numbers that are true now.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResizeReport {
    pub instance_id: InstanceId,
    /// The process that holds the machine; its cgroup and resident memory show the effect.
    pub host_pid: u32,
    /// Whether anything changed live.
    pub applied_live: bool,
    pub cpu: CpuResize,
    pub memory: MemoryResize,
}

impl ResizeReport {
    #[must_use]
    pub fn new(instance_id: InstanceId, outcome: ResizeOutcome) -> Self {
        Self {
            instance_id,
            host_pid: outcome.host_pid,
            applied_live: outcome.applied_live(),
            cpu: outcome.cpu,
            memory: outcome.memory,
        }
    }
}

#[cfg(test)]
mod tests {
    use soma_local::{DimensionState, ResizeOutcome};

    use super::*;

    #[test]
    fn the_document_carries_the_numbers_a_live_script_reads() {
        let outcome = ResizeOutcome {
            host_pid: 4242,
            cpu: CpuResize {
                state: DimensionState::Unchanged,
                requested: None,
                applied: 2,
                booted: 2,
                reason: None,
            },
            memory: MemoryResize {
                state: DimensionState::AppliedLive,
                requested_mb: Some(1536),
                plugged_mb: 512,
                total_mb: 1536,
                booted_mb: 1024,
                ceiling_mb: 3072,
                reason: None,
            },
        };
        let instance = InstanceId::new("0123456789abcdef0123456789abcdef").expect("id");
        let json = serde_json::to_value(ResizeReport::new(instance, outcome)).expect("json");
        assert_eq!(json["applied_live"], true);
        assert_eq!(json["host_pid"], 4242);
        assert_eq!(json["memory"]["state"], "applied_live");
        for (key, value) in [
            ("plugged_mb", 512),
            ("total_mb", 1536),
            ("booted_mb", 1024),
            ("ceiling_mb", 3072),
        ] {
            assert_eq!(json["memory"][key], value, "{key}");
        }
        assert_eq!(json["cpu"]["state"], "unchanged");
    }
}
