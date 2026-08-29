use std::{
    env,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::*;
use super::{
    command::{Arm64Command, Arm64CommandOutcome, Arm64Fixtures, Arm64Terminal},
    executor::execute_arm64_fixture,
};

const UNREACHABLE_SENTINEL: &[u8] = b"SOMA_SENTINEL_NEVER_EMITTED";

#[test]
fn boot_evidence_debug_redacts_guest_console() {
    let evidence = Arm64BootEvidence {
        console: b"guest secret".to_vec(),
    };

    assert_eq!(
        format!("{evidence:?}"),
        "Arm64BootEvidence { console_len: 12 }"
    );
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit kernel/initramfs fixtures"]
fn boots_linux_arm64_pid1_and_releases_descriptors() {
    let kernel = fixture("SOMA_KVM_ARM64_KERNEL");
    let initramfs = fixture("SOMA_KVM_ARM64_INITRAMFS");
    let descriptors_before = open_descriptor_count();
    let started = Instant::now();

    let evidence = boot_arm64_fixture(&kernel, &initramfs)
        .expect("the nested ARM64 KVM guest must reach the fixture PID1 sentinel");
    let elapsed = started.elapsed();
    assert!(
        evidence
            .console()
            .windows(ARM64_BOOT_SENTINEL.len())
            .any(|window| window == ARM64_BOOT_SENTINEL.as_bytes())
    );
    let descriptors_after = open_descriptor_count();
    eprintln!(
        "ARM64 cold boot: elapsed_ms={}, fd_before={descriptors_before}, fd_after={descriptors_after}",
        elapsed.as_millis()
    );
    assert_eq!(descriptors_after, descriptors_before);
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit kernel/initramfs fixtures"]
fn watchdog_stops_a_guest_that_never_emits_the_expected_sentinel() {
    let kernel = fixture("SOMA_KVM_ARM64_KERNEL");
    let initramfs = fixture("SOMA_KVM_ARM64_INITRAMFS");
    let descriptors_before = open_descriptor_count();
    let started = Instant::now();

    let error = boot_with(
        &kernel,
        &initramfs,
        UNREACHABLE_SENTINEL,
        Duration::from_secs(5),
    )
    .unwrap_err();
    let elapsed = started.elapsed();

    assert!(error.to_string().contains("timed out"));
    let descriptors_after = open_descriptor_count();
    eprintln!(
        "ARM64 forced timeout: elapsed_ms={}, fd_before={descriptors_before}, fd_after={descriptors_after}",
        elapsed.as_millis()
    );
    assert_eq!(descriptors_after, descriptors_before);
}

fn fixture(variable: &str) -> PathBuf {
    let path = PathBuf::from(
        env::var_os(variable)
            .unwrap_or_else(|| panic!("{variable} must name an explicit fixture path")),
    );
    assert!(path.is_absolute(), "{variable} must be absolute");
    assert!(path.is_file(), "{variable} must name an existing file");
    path
}

fn open_descriptor_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .expect("the KVM live-test host must mount procfs")
        .count()
}

fn command_fixture() -> (PathBuf, PathBuf) {
    (
        fixture("SOMA_KVM_ARM64_KERNEL"),
        fixture("SOMA_KVM_ARM64_COMMAND_INITRAMFS"),
    )
}

fn execute(
    program: &str,
    args: &[&str],
    timeout: Duration,
    output_limit: usize,
) -> Arm64CommandOutcome {
    let (kernel, initramfs) = command_fixture();
    execute_arm64_fixture(
        Arm64Fixtures {
            kernel: &kernel,
            initramfs: &initramfs,
        },
        Arm64Command {
            program,
            args,
            timeout,
            output_limit,
        },
    )
    .expect("trusted ARM64 command fixture must return an exact terminal frame")
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn executes_arm64_probe_with_exact_argv_and_delayed_small_output() {
    let outcome = execute(
        "/probe",
        &["argv", "", "a b", "$HOME;echo hacked"],
        Duration::from_secs(2),
        4096,
    );
    assert_eq!(outcome.terminal, Arm64Terminal::Exited(0));
    assert_eq!(outcome.stdout, b"/probe\0argv\0\0a b\0$HOME;echo hacked\0");
    assert_eq!(outcome.stderr, b"probe-stderr\n");

    let delayed = execute("/probe", &["delayed"], Duration::from_secs(2), 4096);
    assert_eq!(delayed.terminal, Arm64Terminal::Exited(0));
    assert_eq!(delayed.stdout, b"ab");

    let binary = execute("/probe", &["binary"], Duration::from_secs(2), 256);
    assert_eq!(binary.terminal, Arm64Terminal::Exited(0));
    assert_eq!(binary.stdout, (0..=u8::MAX).collect::<Vec<_>>());
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn reports_arm64_probe_nonzero_exit_and_signal() {
    let exited = execute("/probe", &["exit"], Duration::from_secs(2), 4096);
    assert_eq!(exited.terminal, Arm64Terminal::Exited(7));
    let signaled = execute("/probe", &["signal"], Duration::from_secs(2), 4096);
    assert_eq!(signaled.terminal, Arm64Terminal::Signaled(libc::SIGTERM));
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn reports_guest_deadline_and_cleans_process_group_pipe_holders() {
    for mode in ["sleep", "closed"] {
        let timed_out = execute("/probe", &[mode], Duration::from_millis(250), 4096);
        assert_eq!(timed_out.terminal, Arm64Terminal::TimedOut);
    }
    let descendant = execute("/probe", &["descendant"], Duration::from_secs(2), 4096);
    assert_eq!(descendant.terminal, Arm64Terminal::Exited(0));
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn reports_exact_aggregate_output_limit() {
    let maximum = execute("/probe", &["maximum"], Duration::from_secs(2), 64 * 1024);
    assert_eq!(maximum.terminal, Arm64Terminal::Exited(0));
    assert_eq!(maximum.stdout, vec![b'm'; 64 * 1024]);

    let exact = execute("/probe", &["exact"], Duration::from_secs(2), 1024);
    assert_eq!(exact.terminal, Arm64Terminal::Exited(0));
    assert_eq!(exact.stdout, vec![b'x'; 1024]);

    let outcome = execute("/probe", &["one-over"], Duration::from_secs(2), 1024);
    assert_eq!(outcome.terminal, Arm64Terminal::OutputLimit);
    assert_eq!(outcome.stdout, vec![b'x'; 1024]);
    assert!(outcome.stderr.is_empty());

    let combined = execute("/probe", &["combined"], Duration::from_secs(2), 1024);
    assert_eq!(combined.terminal, Arm64Terminal::OutputLimit);
    assert_eq!(combined.stdout, vec![b'o'; 512]);
    assert_eq!(combined.stderr, vec![b'e'; 512]);
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn reports_execve_failure_without_masquerading_as_exit_127() {
    let outcome = execute("/missing-probe", &[], Duration::from_secs(2), 4096);
    assert_eq!(outcome.terminal, Arm64Terminal::ExecFailed(libc::ENOENT));
}

#[test]
#[ignore = "requires Linux ARM64, /dev/kvm, and explicit command fixtures"]
fn repeated_arm64_commands_release_descriptors_and_tasks() {
    let descriptors = open_descriptor_count();
    let tasks = std::fs::read_dir("/proc/self/task").unwrap().count();
    for _ in 0..3 {
        let outcome = execute("/probe", &["exit"], Duration::from_secs(2), 4096);
        assert_eq!(outcome.terminal, Arm64Terminal::Exited(7));
        assert_eq!(open_descriptor_count(), descriptors);
        assert_eq!(std::fs::read_dir("/proc/self/task").unwrap().count(), tasks);
    }
}
