//! Containment proofs: descriptors, identity, namespaces, root, cgroup, and seccomp phases.

use std::fs;

use soma_jail::{ProbeCommand, namespace_ids_of, own_namespace_ids};

use super::harness::{self, IDENTITY, KVM_ROLES, MEMORY_MAX, PIDS_MAX, ROLES};

const EROFS: i32 = 30;

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn sealed_table_hides_an_injected_descriptor() {
    let live = harness::require();
    // A stray non-close-on-exec descriptor in the launcher must never reach the child.
    let injected = fs::File::open("/etc/hostname").expect("injected descriptor");
    let mut jail = live.launch("sealed", &KVM_ROLES, harness::limits());
    let report = jail.report();
    assert!(
        report.table_sealed,
        "first bad slot {:?}",
        report.first_bad_slot
    );
    assert_eq!(harness::open_fds(jail.handle.pid()), vec![0, 1, 2, 3, 4]);
    assert_eq!(jail.handle.evidence().descriptor_count, 5);
    drop(injected);
    jail.exit(&live, 0);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn child_runs_as_the_ephemeral_identity_in_six_fresh_namespaces() {
    let live = harness::require();
    let mut jail = live.launch("identity", &ROLES, harness::limits());
    let report = jail.report();
    let expected = (IDENTITY.uid, IDENTITY.uid, IDENTITY.gid, IDENTITY.gid);
    assert_eq!((report.uid, report.euid, report.gid, report.egid), expected);
    assert_eq!(
        report.pid, 1,
        "the probe must be PID 1 of its own PID namespace"
    );
    let evidence = jail.handle.evidence().clone();
    assert_eq!(
        (evidence.status.uid, evidence.status.gid),
        (IDENTITY.uid, IDENTITY.gid)
    );
    assert!(evidence.status.no_new_privs);
    assert_eq!(evidence.status.seccomp_mode, 2);
    assert_eq!(evidence.status.capabilities_effective, 0);
    assert_eq!(evidence.status.capabilities_permitted, 0);
    let own = own_namespace_ids().expect("own namespaces");
    let child = namespace_ids_of(jail.handle.pid()).expect("child namespaces");
    assert!(
        child.differs_entirely_from(&own),
        "own {own:?} child {child:?}"
    );
    assert_eq!(evidence.namespaces, child);
    assert_eq!(
        evidence.interface_count, 1,
        "only lo may exist in the network namespace"
    );
    jail.exit(&live, 0);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn root_is_empty_read_only_without_procfs_or_sysfs() {
    let live = harness::require();
    let mut jail = live.launch("root", &ROLES, harness::limits());
    let report = jail.report();
    assert_eq!(report.root.entries, 0, "root must be empty");
    assert!(!report.root.writable);
    assert!(
        !report.root.proc_visible,
        "procfs must not exist after startup"
    );
    assert!(!report.root.sys_visible, "sysfs must not exist");
    assert_eq!(
        jail.expect(ProbeCommand::CreateFile, "ok "),
        format!("ok {EROFS}")
    );
    jail.exit(&live, 0);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn cgroup_limits_read_back_and_contain_the_child() {
    let live = harness::require();
    let limits = harness::limits();
    let mut jail = live.launch("cgroup", &ROLES, limits);
    let _ = jail.report();
    let readback = jail
        .handle
        .cgroup()
        .verify(&limits)
        .expect("limits read back");
    assert_eq!(readback.memory_max, MEMORY_MAX);
    assert_eq!(readback.swap_max, 0);
    assert!(readback.oom_group);
    assert_eq!(readback.pids_max, PIDS_MAX);
    assert_eq!(readback.cpu_max, limits.cpu_max);
    assert_eq!(jail.handle.cgroup().contains(jail.handle.pid()), Ok(true));
    assert_eq!(jail.handle.cgroup().populated(), Ok(true));
    jail.exit(&live, 0);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn forbidden_syscalls_are_recorded_seccomp_kills() {
    let live = harness::require();
    let mut jail = live.launch("socket", &ROLES, harness::limits());
    let _ = jail.report();
    jail.expect_kill(&live, ProbeCommand::Socket);
    let mut jail = live.launch("exec", &ROLES, harness::limits());
    let _ = jail.report();
    jail.expect_kill(&live, ProbeCommand::Exec);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn kvm_version_is_admitted_while_tunsetiff_is_killed() {
    let live = harness::require();
    live.require_kvm();
    let mut jail = live.launch("kvm", &KVM_ROLES, harness::limits());
    let _ = jail.report();
    assert_eq!(jail.expect(ProbeCommand::KvmVersion, "ok "), "ok 12");
    jail.expect_kill(&live, ProbeCommand::ForbiddenIoctl);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn steady_state_keeps_threads_but_drops_setup_syscalls_and_ioctls() {
    let live = harness::require();
    live.require_kvm();
    let mut jail = live.launch("steady-ioctl", &KVM_ROLES, harness::limits());
    let _ = jail.report();
    assert_eq!(jail.expect(ProbeCommand::Steady, "ok "), "ok steady");
    assert_eq!(jail.expect(ProbeCommand::Threads(2), "ok "), "ok 2 0");
    // Retiring the launch page removes its memory slot while the guest runs, so this request
    // must survive into steady state. The KVM descriptor is not a VM descriptor, so the kernel
    // rejects it; any reply at all proves the filter admitted it rather than killing the VMM.
    assert!(
        jail.expect(ProbeCommand::SetMemoryRegion, "ok ")
            .starts_with("ok -1 "),
        "the jail must admit launch-page retirement in steady state",
    );
    jail.expect_kill(&live, ProbeCommand::KvmVersion);

    let mut jail = live.launch("steady-openat", &ROLES, harness::limits());
    let _ = jail.report();
    assert_eq!(jail.expect(ProbeCommand::Steady, "ok "), "ok steady");
    jail.expect_kill(&live, ProbeCommand::CreateFile);
}
