//! Where one sandbox starts from.
//!
//! The two starting points differ in cost by two orders of magnitude, so which one a launch takes
//! is the single most consequential thing about it, and it is worth naming on its own rather than
//! reading it out of a struct field beside the identities.

use soma_kvm::x86_64::{SandboxConfig, SandboxDisks};

/// Where a sandbox starts from.
///
/// Cold boot runs the kernel and userspace init on the request path, which costs hundreds of
/// milliseconds. Restoring resumes a machine already past that point, captured once for the whole
/// Generation, so the request path pays only the resume, the session, and the repair.
pub(in crate::backend::kvm) enum Source {
    /// Build a machine and boot the kernel.
    ColdBoot(SandboxConfig),
    /// Resume the captured machine, giving this Instance its own private head.
    Restore {
        snapshot: std::path::PathBuf,
        disks: SandboxDisks,
        memory_bytes: u64,
    },
}
