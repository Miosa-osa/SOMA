//! Failure and exhaustion proofs: pids and memory limits, stuck children, pidfd identity,
//! parent death, launcher death with recovery, and fail-closed launches.

#![allow(unsafe_code)]

use std::{
    env, fs,
    io::{BufRead, BufReader},
    os::fd::OwnedFd,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use soma_jail::{
    CgroupError, ChildFailure, ChildStep, DescriptorError, DescriptorKind, ExitReason, JailLedger,
    LaunchError, LedgerRecord, ProbeCommand, Resources, SignalError, WaitError, launch,
};

use super::harness::{self, PIDS_MAX, ROLES};

const EAGAIN: i32 = 11;
const HELPER: &str = "failure::helper_launch_and_sleep_until_killed";
const KILLED: ExitReason = ExitReason::Signaled {
    signal: libc::SIGKILL,
    core_dumped: false,
};

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn pids_max_exhaustion_is_contained() {
    let live = harness::require();
    let mut jail = live.launch("pids", &ROLES, harness::limits());
    let _ = jail.report();
    let reply = jail.expect(ProbeCommand::Threads(32), "ok ");
    let fields: Vec<i64> = reply[3..]
        .split(' ')
        .map(|field| field.parse().expect("number"))
        .collect();
    assert!(
        fields[0] < i64::from(PIDS_MAX),
        "{} threads under pids.max {PIDS_MAX}",
        fields[0]
    );
    assert_eq!(fields[1], i64::from(EAGAIN), "clone must fail with EAGAIN");
    jail.exit(&live, 0);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn memory_max_oom_kills_the_whole_group() {
    let live = harness::require();
    let mut limits = harness::limits();
    limits.memory_max_bytes = 32 << 20;
    let mut jail = live.launch("oom", &ROLES, limits);
    let _ = jail.report();
    jail.expect_silence(ProbeCommand::Allocate(96));
    let evidence = jail.finish(&live, KILLED);
    assert!(
        evidence.oom_kills >= 1,
        "memory.events must record the OOM kill"
    );
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn stuck_child_is_killed_through_the_pidfd_at_reconcile() {
    let live = harness::require();
    let mut jail = live.launch("stuck", &ROLES, harness::limits());
    let _ = jail.report();
    let short = Instant::now() + Duration::from_millis(300);
    assert_eq!(jail.handle.wait(short), Err(WaitError::Timeout));
    let record = jail.handle.ledger().record();
    let (disposition, evidence) = jail.handle.reconcile(harness::deadline(5));
    assert!(disposition.is_released(), "{disposition}");
    assert_eq!(evidence.exit, Some(KILLED));
    live.assert_zero_residual(&record);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn pidfd_identity_outlives_the_numeric_pid() {
    let live = harness::require();
    let mut jail = live.launch("pidfd", &ROLES, harness::limits());
    let _ = jail.report();
    assert_eq!(jail.handle.signal(0), Ok(()));
    jail.control.send(ProbeCommand::Exit(3));
    assert_eq!(
        jail.handle.wait(harness::deadline(5)),
        Ok(ExitReason::Exited(3))
    );
    assert_eq!(jail.handle.signal(0), Err(SignalError::Gone));
    assert_eq!(jail.handle.kill(), Err(SignalError::Gone));
    jail.finish(&live, ExitReason::Exited(3));
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn launching_thread_exit_kills_the_child() {
    let live = harness::require();
    // The report is consumed inside the launching thread so the socket is empty afterwards.
    let launcher = || {
        let mut jail = live.launch("pdeathsig-thread", &ROLES, harness::limits());
        let _ = jail.report();
        jail
    };
    let jail = thread::scope(|scope| scope.spawn(launcher).join().expect("launcher thread"));
    assert_eq!(
        jail.control.recv(Duration::from_millis(100)),
        None,
        "probe must be dead"
    );
    jail.finish(&live, KILLED);
}

/// Runs only as the helper of `launcher_process_death_kills_the_child_and_recovery_releases`.
#[test]
#[ignore = "helper for the launcher-death test; never a test on its own"]
fn helper_launch_and_sleep_until_killed() {
    assert_eq!(
        env::var("SOMA_JAIL_HELPER").as_deref(),
        Ok("1"),
        "helper invoked directly"
    );
    let live = harness::require();
    let mut jail = live.launch("pdeathsig-process", &ROLES, harness::limits());
    let _ = jail.report();
    let record = jail.handle.ledger().record();
    let root = record.jail_root.display();
    println!(
        "\njail pid={} leaf={} root={root}",
        jail.handle.pid(),
        record.leaf
    );
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn reap_orphan(pid: i32, deadline: Instant) -> (i32, i32) {
    loop {
        // SAFETY: `siginfo` is zeroed storage the kernel fills on success.
        let mut siginfo: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let id = libc::id_t::try_from(pid).expect("pid");
        let options = libc::WEXITED | libc::WNOHANG;
        // SAFETY: `P_PID` names the reparented child and `siginfo` is valid writable storage.
        let result = unsafe { libc::waitid(libc::P_PID, id, &raw mut siginfo, options) };
        assert_eq!(result, 0, "waitid: {}", std::io::Error::last_os_error());
        // SAFETY: `si_pid` and `si_status` are valid after a successful `WEXITED` wait.
        if unsafe { siginfo.si_pid() } == pid {
            // SAFETY: as above.
            return (siginfo.si_code, unsafe { siginfo.si_status() });
        }
        assert!(
            Instant::now() < deadline,
            "jailed child {pid} outlived its launcher"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn launcher_process_death_kills_the_child_and_recovery_releases() {
    let live = harness::require();
    // SAFETY: `PR_SET_CHILD_SUBREAPER` takes integer arguments only; orphans reparent here.
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
        0
    );
    let mut helper = Command::new(env::current_exe().expect("test binary"))
        .args([
            "--exact",
            HELPER,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SOMA_JAIL_HELPER", "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("helper");
    let stdout = BufReader::new(helper.stdout.take().expect("helper stdout"));
    let announcement = stdout
        .lines()
        .map_while(Result::ok)
        .find_map(|candidate| {
            candidate
                .find("jail pid=")
                .map(|at| candidate[at..].to_owned())
        })
        .expect("helper announced its jail");
    let field = |key: &str| {
        let value = announcement
            .split(' ')
            .find_map(|part| part.strip_prefix(key));
        value.expect(key).to_owned()
    };
    let pid: i32 = field("pid=").parse().expect("pid");
    let record = LedgerRecord {
        leaf: field("leaf="),
        jail_root: field("root=").into(),
        pid: Some(pid),
    };
    assert!(
        fs::metadata(format!("/proc/{pid}")).is_ok(),
        "jailed child is alive before"
    );
    helper.kill().expect("kill helper");
    helper.wait().expect("reap helper");
    let child_exit = reap_orphan(pid, harness::deadline(5));
    assert_eq!(
        child_exit,
        (libc::CLD_KILLED, libc::SIGKILL),
        "must die of the death signal"
    );
    let disposition = JailLedger::recover(&live.cgroup_root, &record, harness::deadline(5));
    assert!(disposition.is_released(), "{disposition}");
    live.assert_zero_residual(&record);
}

fn expect_verify_failure(
    live: &harness::Live,
    name: &str,
    resources: Resources,
    expected: DescriptorError,
) {
    let spec = harness::spec(name, &ROLES, harness::limits());
    let failure = launch(&spec, &live.anchors(), resources).expect_err("must fail closed");
    let step = ChildStep::Verify(expected);
    assert_eq!(
        failure.error,
        LaunchError::Child(ChildFailure { step, errno: 0 })
    );
    assert!(failure.cleanup.is_released(), "{}", failure.cleanup);
    live.assert_zero_residual(&LedgerRecord {
        leaf: spec.leaf.as_str().to_owned(),
        jail_root: live.jail_root_parent.join(spec.leaf.as_str()),
        pid: None,
    });
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn wrong_descriptor_kinds_fail_closed_before_seccomp() {
    let live = harness::require();
    let (mut resources, _control) = live.resources(&ROLES);
    resources.descriptors[0].1 = fs::File::open("/etc/hostname")
        .expect("regular file")
        .into();
    let found = Some(DescriptorKind::RegularFile);
    let expected = DescriptorError::Kind { slot: 3, found };
    expect_verify_failure(&live, "inject-file", resources, expected);

    let (mut resources, _control) = live.resources(&ROLES);
    let (stream, _peer) = stream_pair();
    resources.descriptors[0].1 = stream;
    let expected = DescriptorError::NotSeqpacket { slot: 3 };
    expect_verify_failure(&live, "inject-stream", resources, expected);
}

#[test]
#[ignore = "privileged live test; run scripts/jail-live-tests.sh"]
fn an_existing_leaf_fails_closed_and_is_never_reused() {
    let live = harness::require();
    let spec = harness::spec("existing-leaf", &ROLES, harness::limits());
    let leaf = live.cgroup_root.join(spec.leaf.as_str());
    fs::create_dir(&leaf).expect("pre-existing leaf");
    let (resources, _control) = live.resources(&ROLES);
    let failure = launch(&spec, &live.anchors(), resources).expect_err("must fail closed");
    assert_eq!(
        failure.error,
        LaunchError::Cgroup(CgroupError::AlreadyExists)
    );
    assert!(failure.cleanup.is_released());
    assert!(
        leaf.is_dir(),
        "a leaf the launcher does not own must not be removed"
    );
    fs::remove_dir(&leaf).expect("remove pre-existing leaf");
}

fn stream_pair() -> (OwnedFd, OwnedFd) {
    let (left, right) = std::os::unix::net::UnixStream::pair().expect("stream pair");
    (left.into(), right.into())
}
