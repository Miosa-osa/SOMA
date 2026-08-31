//! The jail around the real `soma-vmm` worker.
//!
//! These tests exec the production binary rather than the `jail-probe` stand-in, so what they
//! prove is that the worker itself attests its containment, serves the lifecycle contract from
//! inside the jail, narrows its own filter, and leaves nothing behind.

use soma_jail::{ExitReason, ProbeReport, namespace_ids_of, own_namespace_ids};
use soma_vmm::{
    Argument, DiskBytes, Execute, ExecutionLimits, Generation, GenerationId, InstanceId, Launch,
    MachineSpec, MemoryBytes, OperationId, OutputBytes, Program, Stop, TimeoutMillis, VcpuCount,
    control::Request,
};

use super::{
    control::Jail,
    harness::{self, IDENTITY, MEMORY_MAX, PIDS_MAX, ROLES},
};

/// The status the worker leaves with when its control socket reaches end of stream; the
/// worker's own `exit` module states the contract this mirrors.
const SUPERVISOR_GONE: i32 = 7;

/// The launch the supervisor asks for; this worker restores no machine, so the Generation it
/// names only has to be a well-formed one.
fn launch_request() -> Request {
    let machine = MachineSpec::new(
        VcpuCount::new(1).expect("vcpus"),
        MemoryBytes::new(1 << 30).expect("memory"),
        DiskBytes::new(1 << 32).expect("disk"),
    );
    Request::Launch(Launch::new(
        OperationId::new([7; 16]).expect("operation"),
        InstanceId::new([8; 16]).expect("instance"),
        Generation::new(GenerationId::new([9; 32]).expect("generation"), machine),
    ))
}

/// One bounded command, on its own operation identifier so the Machine judges it by its
/// lifecycle rather than by a conflict with the Launch above.
fn execute_request() -> Request {
    let limits = ExecutionLimits::new(
        TimeoutMillis::new(1_000).expect("timeout"),
        OutputBytes::new(4_096).expect("output"),
    );
    Request::Execute(
        Execute::new(
            OperationId::new([10; 16]).expect("operation"),
            InstanceId::new([8; 16]).expect("instance"),
            Program::new(b"/bin/true".to_vec()).expect("program"),
            vec![Argument::new(b"--version".to_vec()).expect("argument")],
            limits,
        )
        .expect("execute"),
    )
}

fn stop_request() -> Request {
    Request::Stop(Stop::new(
        OperationId::new([11; 16]).expect("operation"),
        InstanceId::new([8; 16]).expect("instance"),
    ))
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn the_worker_attests_the_jail_it_was_launched_into() {
    let live = harness::require();
    let limits = harness::limits();
    let mut jail = live.launch_vmm("vmm-attest", &ROLES, limits);

    let report = jail.report();
    assert!(
        report.table_sealed,
        "first bad slot {:?}",
        report.first_bad_slot
    );
    let expected = (IDENTITY.uid, IDENTITY.uid, IDENTITY.gid, IDENTITY.gid);
    assert_eq!((report.uid, report.euid, report.gid, report.egid), expected);
    assert_eq!(report.pid, 1, "the worker must be PID 1 of its namespace");
    assert_eq!(report.root.entries, 0, "root must be empty");
    assert!(!report.root.writable);
    assert!(!report.root.proc_visible);
    assert!(!report.root.sys_visible);

    // Only the standard streams and the one manifested control socket are open.
    assert_eq!(harness::open_fds(jail.handle.pid()), vec![0, 1, 2, 3]);
    let evidence = jail.handle.evidence().clone();
    assert_eq!(evidence.descriptor_count, 4);
    assert!(evidence.status.no_new_privs);
    assert_eq!(evidence.status.seccomp_mode, 2, "the filter is installed");
    assert_eq!(evidence.status.capabilities_effective, 0);
    assert_eq!(evidence.status.capabilities_permitted, 0);
    let own = own_namespace_ids().expect("own namespaces");
    let child = namespace_ids_of(jail.handle.pid()).expect("worker namespaces");
    assert!(
        child.differs_entirely_from(&own),
        "own {own:?} worker {child:?}"
    );
    assert_eq!(evidence.namespaces, child);
    assert_eq!(evidence.interface_count, 1, "only lo may exist");

    let readback = jail
        .handle
        .cgroup()
        .verify(&limits)
        .expect("limits read back");
    assert_eq!(readback.memory_max, MEMORY_MAX);
    assert_eq!(readback.pids_max, PIDS_MAX);
    assert_eq!(jail.handle.cgroup().contains(jail.handle.pid()), Ok(true));

    jail.control.send_text(&Request::Shutdown(0).encode());
    jail.finish(&live, ExitReason::Exited(0));
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn the_worker_serves_the_lifecycle_contract_from_inside_the_jail() {
    let live = harness::require();
    let mut jail = live.launch_vmm("vmm-lifecycle", &ROLES, harness::limits());
    let _ = jail.report();

    // This worker owns no platform, so the honest outcome of a Launch is the typed failure of
    // the verification it cannot perform, with its milestones and its rollback evidence.
    let launch = launch_request().encode();
    let failed = jail.expect_text(&launch, "failure ");
    assert_eq!(
        failed,
        "failure kind=GenerationVerificationFailed phase=ArtifactVerification \
         recovery=RepairHost cleanup=Complete \
         milestones=RequestAccepted,RollbackStarted,CleanupCompleted"
    );
    // The same operation replays its own receipt rather than being performed again.
    assert_eq!(jail.expect_text(&launch, "failure "), failed);

    // Execute is refused by the lifecycle rather than attempted after a failed Launch.
    assert_eq!(
        jail.expect_text(&execute_request().encode(), "failure "),
        "failure kind=InvalidLifecycle phase=Lifecycle recovery=DoNotRetry \
         cleanup=NotRequired milestones=none"
    );

    // A packet the protocol does not define changes nothing.
    assert_eq!(
        jail.expect_text("mount /", "rejected "),
        "rejected unknown request"
    );

    assert_eq!(
        jail.expect_text(&stop_request().encode(), "failure "),
        "failure kind=StopFailed phase=Stop recovery=RepairHost cleanup=Incomplete \
         milestones=StopRequested"
    );

    jail.control.send_text(&Request::Shutdown(0).encode());
    jail.finish(&live, ExitReason::Exited(0));
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn sealing_the_filter_turns_a_later_attestation_into_a_seccomp_kill() {
    let live = harness::require();
    let mut jail = live.launch_vmm("vmm-seal", &ROLES, harness::limits());
    let first = jail.report();

    // Under the startup filter the worker can attest again and sees exactly what it saw.
    let repeated = jail.expect_text(&Request::Attest.encode(), "pid=");
    assert_eq!(ProbeReport::decode(&repeated), Ok(first));
    assert_eq!(
        jail.expect_text(&Request::Seal.encode(), "sealed"),
        "sealed"
    );

    // The steady-state filter drops the startup-only syscalls attestation needs, so the same
    // request the worker just served is now a kill rather than a degraded answer.
    jail.expect_kill_text(&live, &Request::Attest.encode());
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn the_worker_leaves_when_its_supervisor_closes_the_control_socket() {
    let live = harness::require();
    let mut jail = live.launch_vmm("vmm-eof", &ROLES, harness::limits());
    let _ = jail.report();

    let Jail {
        mut handle,
        control,
    } = jail;
    drop(control);
    // Losing a supervisor is its own exit status, never the clean one an ordered stop uses.
    let gone = ExitReason::Exited(SUPERVISOR_GONE);
    assert_eq!(
        handle.wait(harness::deadline(5)),
        Ok(gone),
        "the worker must leave when nobody is left to serve"
    );
    let record = handle.ledger().record();
    let (disposition, evidence) = handle.reconcile(harness::deadline(5));
    assert!(disposition.is_released(), "{disposition}");
    assert_eq!(evidence.exit, Some(gone));
    live.assert_zero_residual(&record);
}
