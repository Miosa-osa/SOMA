use super::*;
use crate::{
    DeclaredDevices, DiskBytes, Executed, Generation, GenerationId, InstanceId, Launch, Machine,
    MachineSpec, MemoryBytes, Stopped, VcpuCount,
    control::Reply,
    {CleanupEvidence, Milestone, Milestones},
};

fn operation() -> OperationId {
    OperationId::new([1; 16]).expect("operation")
}

fn instance() -> InstanceId {
    InstanceId::new([2; 16]).expect("instance")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn workspace_outcomes_reject_uncertain_or_unknown_receipts() {
    use soma_guest::WorkspaceOutcome;
    for result in [
        WorkspaceOutcome::Ready,
        WorkspaceOutcome::Refused,
        WorkspaceOutcome::Unsupported,
    ] {
        let reply = Reply::Workspace(instance(), result).encode();
        assert_eq!(
            Outcome::decode(&reply),
            Ok(Outcome::Workspace(instance(), result))
        );
        assert!(Outcome::decode(&(reply + " extra")).is_err());
    }
    assert!(
        Outcome::decode(&Reply::Workspace(instance(), WorkspaceOutcome::Inconsistent).encode())
            .is_err()
    );
}

fn launch() -> Launch {
    let machine = MachineSpec::new(
        VcpuCount::new(1).expect("vcpus"),
        MemoryBytes::new(1 << 30).expect("memory"),
        DiskBytes::new(1 << 32).expect("disk"),
    );
    Launch::new(
        operation(),
        instance(),
        Generation::new(
            GenerationId::new([3; 32]).expect("generation"),
            machine,
            DeclaredDevices::new(true, true),
        ),
    )
}

/// Every form the worker can encode is a form the supervisor recovers the same values from.
#[test]
fn every_reply_the_worker_sends_decodes_to_what_it_meant() {
    assert_eq!(
        Outcome::decode(&Reply::Paused(instance()).encode()),
        Ok(Outcome::Paused(instance()))
    );
    assert_eq!(
        Outcome::decode(&Reply::Resumed(instance()).encode()),
        Ok(Outcome::Resumed(instance()))
    );
    let executed = Executed::new(
        operation(),
        instance(),
        ExitStatus::Code(0),
        b"soma-ok\n".to_vec(),
        Vec::new(),
    );
    assert_eq!(
        Outcome::decode(&Reply::Executed(&executed).encode()),
        Ok(Outcome::Executed {
            operation_id: operation(),
            status: ExitStatus::Code(0),
            stdout_bytes: 8,
            stderr_bytes: 0,
        })
    );

    let mut milestones = Milestones::default();
    milestones.push(Milestone::Stopped);
    let stopped = Stopped::new(
        operation(),
        instance(),
        true,
        false,
        CleanupEvidence::Complete,
        milestones,
    );
    assert_eq!(
        Outcome::decode(&Reply::Stopped(&stopped).encode()),
        Ok(Outcome::Stopped {
            guest_acknowledged: true,
            forced: false,
        })
    );

    let failure = Machine::new().launch(launch()).expect_err("no platform");
    let Ok(Outcome::Failure { kind, .. }) = Outcome::decode(&Reply::Failure(&failure).encode())
    else {
        panic!("a failure reply must decode as a failure");
    };
    assert_eq!(kind, crate::FailureKind::GenerationVerificationFailed);

    assert_eq!(
        Outcome::decode(&Reply::Sealed.encode()),
        Ok(Outcome::Sealed)
    );
    assert_eq!(
        Outcome::decode(&Reply::Output(b"\x00\xff").encode()),
        Ok(Outcome::Output(vec![0, 255]))
    );
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    assert_eq!(
        Outcome::decode(
            &Reply::Pty(&soma::PtyAnswer::Output {
                bytes: vec![0, 255],
                end: false,
            })
            .encode()
        ),
        Ok(Outcome::Pty(soma::PtyAnswer::Output {
            bytes: vec![0, 255],
            end: false,
        }))
    );
    assert_eq!(
        Outcome::decode(&Reply::Rejected("unknown request").encode()),
        Ok(Outcome::Rejected("unknown request".to_owned()))
    );
}

#[test]
fn a_reply_this_contract_does_not_define_is_refused() {
    assert_eq!(
        Outcome::decode("mounted /"),
        Err(ControlError::UnknownRequest)
    );
    assert_eq!(
        Outcome::decode("executed operation=01 status=code:0 stdout=0 stderr=0"),
        Err(ControlError::InvalidValue("operation"))
    );
    assert_eq!(
        Outcome::decode("stopped acknowledged=maybe forced=false"),
        Err(ControlError::InvalidValue("acknowledged"))
    );
    assert_eq!(
        Outcome::decode("failure kind=NoSuchKind"),
        Err(ControlError::InvalidValue("kind"))
    );
}
