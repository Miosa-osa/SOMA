//! What one sandbox is built from, and where it starts.
//!
//! The two starting points differ in cost by two orders of magnitude, so which one a launch takes
//! is the single most consequential thing about it, and it is worth naming on its own rather than
//! reading it out of a struct field beside the identities.

use soma_guest::{LaunchNetwork, SecretFile};
use soma_kvm::DeviceSet;
use soma_kvm::x86_64::{
    Hypervisor, NetworkAttachment, SandboxConfig, SandboxDisks, SnapshotObjects,
};

use super::pending::PendingActivation;

/// Where a sandbox starts from.
///
/// Cold boot runs the kernel and userspace init on the request path, which costs hundreds of
/// milliseconds. Restoring resumes a machine already past that point, captured once for the whole
/// Generation, so the request path pays only the resume, the session, and the repair.
pub enum Source {
    /// Build a machine and boot the kernel.
    ColdBoot(SandboxConfig),
    /// Resume the captured machine, giving this Instance its own private head.
    ///
    /// The snapshot arrives as open handles rather than as a directory, so a machine with no
    /// filesystem of its own can resume from exactly the objects its broker opened.
    Restore {
        objects: SnapshotObjects,
        /// Where this machine's `/dev/kvm` handle comes from.
        hypervisor: Hypervisor,
        disks: SandboxDisks,
        /// The optional devices the Generation declared, which the snapshot must agree with.
        devices: DeviceSet,
        memory_bytes: u64,
    },
}

/// Everything one sandbox needs before it can boot.
pub struct Boot {
    /// How this sandbox comes into existence.
    pub source: Source,
    pub generation: [u8; 32],
    pub instance: [u8; 16],
    /// The operation this launch belongs to, bound into the launch page.
    pub operation: [u8; 16],
    /// The vsock context identifier this Instance is assigned.
    ///
    /// Context identifiers are host global, so every concurrent sandbox needs its own. One
    /// command line invocation serves one sandbox, so there is no shared counter to draw from
    /// and the identifier is derived from the Instance identity instead.
    pub guest_cid: u32,
    /// The network this Instance was given.
    pub network: Network,
    /// The secrets this one Instance is launched with.
    ///
    /// They belong to the Boot rather than to the Generation because the Generation, its
    /// artifacts, and the snapshot every Instance of it restores from are shared. A value placed
    /// here reaches one machine over one session and is never part of anything a second Instance
    /// can read.
    pub secrets: Vec<SecretFile>,
}

/// The network one machine is built with.
///
/// The launch values are always present, because the guest repairs an interface either way; the
/// frame path and the activation are present only for an Instance the broker leased a bundle to.
pub struct Network {
    /// The values the launch page carries.
    pub launch: LaunchNetwork,
    /// The assigned frame path, attached with the link still down.
    pub attachment: Option<NetworkAttachment>,
    /// What the repaired session must mint before the broker will let traffic flow.
    pub activation: Option<PendingActivation>,
}
