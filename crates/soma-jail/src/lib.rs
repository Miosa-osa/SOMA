//! The SOMA VMM jail: the launcher that constrains one `soma-vmm` process.
//!
//! Portable types (`JailSpec`, the descriptor manifest, the seccomp policy and BPF assembler,
//! evidence, and the probe report codec) compile on every workspace target.
//! The Linux `x86_64` mechanisms (namespaces, cgroup v2, descriptor sealing, seccomp
//! installation, the launcher, and reconciliation) compile only on the production target.

#![deny(warnings)]
#![deny(unsafe_code)]

mod evidence;
mod manifest;
mod probe;
mod report;
mod seccomp;
mod spec;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cgroup;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod descriptors;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod namespaces;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod process;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod reconcile;

pub use evidence::{ExitReason, JailEvidence, NamespaceIds, ProcessStatus, SIGSYS};
pub use manifest::{
    ArtifactKind, DescriptorKind, DescriptorManifest, DescriptorRole, FIRST_MANIFEST_SLOT,
    ManifestError, STANDARD_STREAMS,
};
pub use probe::ProbeCommand;
pub use report::{ProbeReport, ReportError, RootView};
pub use seccomp::{
    ArgCheck, CLONE_NAMESPACE_MASK, CLONE_THREAD, FilterProgram, Instruction, IoctlPhases,
    IoctlRule, NEVER_ALLOWED, Need, PROT_EXEC, Phase, Provenance, STARTUP_ONLY, STEADY_REQUESTS,
    Source, SyscallRule, TUNSETIFF, ioctl_rules, program_for, startup_only_syscalls, syscall_rules,
};
pub use spec::{CgroupLimits, CpuMax, Identity, IoMax, JailSpec, LeafName, Rlimits, SpecError};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use cgroup::{CgroupError, CgroupLeaf, CgroupReadback};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use descriptors::{
    DescriptorError, KVM_DEVICE, TAP_DEVICE, VerificationDepth, launcher_sealed_len, report_slot,
    verify_sealed_table,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use namespaces::{
    NamespaceError, RootStep, interfaces_of, namespace_ids_of, own_namespace_ids, process_status,
    verify_interfaces,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use process::{
    ChildFailure, ChildStep, HostAnchors, JailHandle, LaunchError, LaunchFailure, Resources,
    RlimitKind, SignalError, WaitError, launch,
};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use reconcile::{Disposition, JailLedger, LedgerRecord, Residual, ResidualKind};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub use seccomp::{SeccompError, install_filter};
