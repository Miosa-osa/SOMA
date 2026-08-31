use crate::{Executed, Failure, Milestone, Ready, Stopped};

use super::field::hex;

/// One reply packet a jailed worker sends back.
///
/// A reply borrows the receipt the Machine produced rather than copying it, so the wire form
/// can never disagree with the contract value it reports.
/// Contract enumerations are named by their variant, which is also how
/// [`Failure`](crate::Failure) displays itself.
#[derive(Clone, Copy, Debug)]
pub enum Reply<'a> {
    Ready(&'a Ready),
    Executed(&'a Executed),
    Stopped(&'a Stopped),
    Failure(&'a Failure),
    /// The seccomp filter now denies every startup-only syscall.
    Sealed,
    /// The request was not performed and Machine state did not change.
    Rejected(&'a str),
}

impl Reply<'_> {
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            Self::Ready(ready) => format!(
                "ready operation={} instance={} generation={} milestones={}",
                hex(ready.operation_id().as_bytes()),
                hex(ready.instance_id().as_bytes()),
                hex(ready.generation_id().as_bytes()),
                milestones(ready.milestones()),
            ),
            Self::Executed(executed) => format!(
                "executed operation={} instance={} status={:?} stdout={} stderr={}",
                hex(executed.operation_id().as_bytes()),
                hex(executed.instance_id().as_bytes()),
                executed.status(),
                executed.stdout().len(),
                executed.stderr().len(),
            ),
            Self::Stopped(stopped) => format!(
                "stopped operation={} instance={} acknowledged={} forced={} cleanup={:?} \
                 milestones={}",
                hex(stopped.operation_id().as_bytes()),
                hex(stopped.instance_id().as_bytes()),
                stopped.guest_acknowledged(),
                stopped.forced(),
                stopped.cleanup(),
                milestones(stopped.milestones()),
            ),
            Self::Failure(failure) => format!(
                "failure kind={:?} phase={:?} recovery={:?} cleanup={:?} milestones={}",
                failure.kind(),
                failure.phase(),
                failure.recovery(),
                failure.cleanup(),
                milestones(failure.milestones()),
            ),
            Self::Sealed => "sealed".to_owned(),
            Self::Rejected(reason) => format!("rejected {reason}"),
        }
    }
}

/// The milestone trail, comma separated, or `none` when the outcome reached no milestone.
fn milestones(reached: &[Milestone]) -> String {
    if reached.is_empty() {
        return "none".to_owned();
    }
    reached
        .iter()
        .map(|milestone| format!("{milestone:?}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DiskBytes, Generation, GenerationId, InstanceId, Launch, Machine, MachineSpec, MemoryBytes,
        OperationId, VcpuCount,
    };

    fn launch() -> Launch {
        let machine = MachineSpec::new(
            VcpuCount::new(1).expect("vcpus"),
            MemoryBytes::new(1 << 30).expect("memory"),
            DiskBytes::new(1 << 32).expect("disk"),
        );
        Launch::new(
            OperationId::new([1; 16]).expect("operation"),
            InstanceId::new([2; 16]).expect("instance"),
            Generation::new(GenerationId::new([3; 32]).expect("generation"), machine),
        )
    }

    #[test]
    fn a_failure_reply_names_its_kind_phase_and_recovery() {
        let failure = Machine::new().launch(launch()).expect_err("no platform");
        assert_eq!(
            Reply::Failure(&failure).encode(),
            "failure kind=GenerationVerificationFailed phase=ArtifactVerification \
             recovery=RepairHost cleanup=Complete \
             milestones=RequestAccepted,RollbackStarted,CleanupCompleted"
        );
    }

    #[test]
    fn fixed_replies_are_single_words() {
        assert_eq!(Reply::Sealed.encode(), "sealed");
        assert_eq!(
            Reply::Rejected("unknown request").encode(),
            "rejected unknown request"
        );
    }
}
