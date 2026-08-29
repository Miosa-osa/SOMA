//! Host-side helpers for the live sandbox proof: prerequisites, scratch space, descriptor and
//! thread accounting, and the assertions every successful run must satisfy.

use std::{fs, path::PathBuf};

use soma_guest::TerminalStatus;
use soma_kvm::x86_64::{LAUNCH_PAGE_GPA, Milestone, SandboxEvidence};

use crate::x86_64_sandbox_boot_session as session;

pub fn require_kvm() {
    let ok = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok();
    assert!(
        ok,
        "prerequisite failed: this live test needs a readable and writable /dev/kvm; it never passes silently"
    );
}

pub fn open_descriptor_count() -> usize {
    fs::read_dir("/proc/self/fd")
        .expect("the KVM live-test host must mount procfs")
        .count()
}

pub fn thread_count() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Threads:"))
                .and_then(|rest| rest.trim().parse().ok())
        })
        .unwrap_or(0)
}

pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("x86_64-sandbox-boot")
        .join(name);
    fs::create_dir_all(&dir).expect("create scratch directory under target/");
    dir
}

/// Everything one successful run must leave behind.
pub struct Proof {
    pub evidence: SandboxEvidence,
    pub hostile: session::Executed,
    pub executed: session::Executed,
    pub fd_before: usize,
    pub fd_after: usize,
    pub threads_before: u64,
    pub threads_after: u64,
    pub root_before: String,
    pub root_after: String,
    pub head_before: String,
    pub head_after: String,
}

pub fn assert_proof(proof: &Proof) {
    session::assert_orderly(&proof.evidence);
    // The hostile step ran first: PID 1 bounded it at the exact allowance, killed its process
    // group, and then accepted the next lifecycle operation on the same authenticated session.
    assert_eq!(proof.hostile.status, TerminalStatus::OutputLimit);
    let hostile_bytes = proof.hostile.stdout.len() + proof.hostile.stderr.len();
    assert_eq!(
        u64::try_from(hostile_bytes).unwrap(),
        session::HOSTILE_ALLOWANCE,
        "the hostile step must deliver exactly its allowance"
    );
    assert!(
        !proof.hostile.stdout.is_empty() && !proof.hostile.stderr.is_empty(),
        "both hostile pipes must have competed"
    );
    assert_eq!(proof.executed.status, TerminalStatus::Exited(0));
    assert_eq!(
        proof.fd_after, proof.fd_before,
        "the sandbox leaked descriptors"
    );
    assert_eq!(
        proof.threads_after, proof.threads_before,
        "the sandbox leaked threads"
    );
    assert_eq!(
        proof.root_before, proof.root_after,
        "the EROFS root changed"
    );
    assert_ne!(
        proof.head_before, proof.head_after,
        "the overlay head never changed"
    );
    assert_eq!(LAUNCH_PAGE_GPA, soma_guest::LAUNCH_PAGE_GUEST_ADDRESS);
    assert_eq!(soma_kvm::SOMA_CONTROL_PORT, soma_guest::CONTROL_VSOCK_PORT);
    let ready = proof.evidence.at(Milestone::Ready).unwrap();
    let start = proof.evidence.at(Milestone::RunStart).unwrap();
    assert!(ready > start);
}
