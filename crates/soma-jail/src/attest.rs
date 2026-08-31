//! What a jailed process can observe about its own containment.
//!
//! The observation uses only syscalls the startup filter admits: `fstat` through the
//! descriptor verifier, the identity getters, and one enumeration of the empty root.
//! Both the `jail-probe` stand-in and the real `soma-vmm` worker attest through this one
//! implementation, so the launcher reads the same evidence from either child.
//!
//! Nothing here survives the steady-state filter, which drops the startup-only syscalls the
//! root view needs; that is deliberate, because a second attestation after the phase
//! transition must be a `SIGSYS` kill rather than a quietly degraded report.

#![allow(unsafe_code)]

use std::fs;

use crate::{
    descriptors::{DescriptorError, VerificationDepth, verify_sealed_table},
    manifest::DescriptorManifest,
    report::{ProbeReport, RootView},
};

/// The file the attestation tries to create in the root; the jail root is read-only, so the
/// attempt must always fail.
const WRITE_PROBE: &str = "/soma-attest";
/// The highest slot the sealed-table scan looks at, whatever `RLIMIT_NOFILE` allows.
const SCAN_CEILING: u32 = 4096;

/// The scan bound: the descriptor limit, never above [`SCAN_CEILING`].
fn descriptor_limit() -> u32 {
    let mut rlimit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `RLIMIT_NOFILE` is a valid resource and `rlimit` outlives the call.
    unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut rlimit) };
    u32::try_from(rlimit.rlim_cur)
        .unwrap_or(u32::MAX)
        .min(SCAN_CEILING)
}

/// The slot a verification failure names, or `u32::MAX` for a whole-table failure.
fn failing_slot(error: DescriptorError) -> u32 {
    match error {
        DescriptorError::Missing { slot, .. }
        | DescriptorError::Kind { slot, .. }
        | DescriptorError::Device { slot }
        | DescriptorError::NotSeqpacket { slot }
        | DescriptorError::Unexpected { slot }
        | DescriptorError::Dup { slot, .. } => slot,
        DescriptorError::CloseRange(_) => u32::MAX,
    }
}

fn root_view() -> RootView {
    let entries = fs::read_dir("/").map_or(u32::MAX, |entries| {
        u32::try_from(entries.count()).unwrap_or(u32::MAX)
    });
    let writable = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(WRITE_PROBE)
        .is_ok();
    RootView {
        entries,
        writable,
        proc_visible: fs::metadata("/proc").is_ok(),
        sys_visible: fs::metadata("/sys").is_ok(),
    }
}

/// Observes the sealed descriptor table, the effective identity, and the root filesystem.
#[must_use]
pub fn attest(manifest: &DescriptorManifest) -> ProbeReport {
    let verification = verify_sealed_table(manifest, VerificationDepth::Sealed, descriptor_limit());
    let first_bad_slot = verification.err().map(failing_slot);
    // SAFETY: the identity getters take no arguments and cannot fail.
    let identity = unsafe {
        [
            libc::getuid(),
            libc::geteuid(),
            libc::getgid(),
            libc::getegid(),
        ]
    };
    ProbeReport {
        // SAFETY: `getpid` takes no arguments and cannot fail.
        pid: unsafe { libc::getpid() },
        uid: identity[0],
        euid: identity[1],
        gid: identity[2],
        egid: identity[3],
        table_sealed: first_bad_slot.is_none(),
        first_bad_slot,
        root: root_view(),
    }
}
