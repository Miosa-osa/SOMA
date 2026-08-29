//! One typed, redacted reason per compatibility invariant a manifest can fail.
//!
//! Every reason names the invariant, never the value that violated it, so a rejection can be
//! reported to an untrusted caller without echoing manifest bytes back.

use core::fmt;

/// The exact compatibility invariant a decoded manifest failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Incompatibility {
    /// The compiler policy version is not the one this profile implements.
    PolicyVersion,
    /// The source platform is not the supported one.
    SourcePlatform,
    /// A bound content digest is the reserved all-zero value.
    ZeroDigest,
    /// The normalized-tree size is zero or beyond the decoder bound.
    TreeSize,
    /// The root filesystem UUID is not derived from the bound tree digest.
    RootUuid,
    /// The root format profile or formatter revision is not the pinned one.
    RootFormat,
    /// The root image size is zero, unaligned, or beyond the profile bound.
    RootSize,
    /// The overlay UUID derivation version or feature profile is not the pinned one.
    OverlayProfile,
    /// The template list is empty, or a capacity is unaligned, undersized, or unknown.
    OverlayCapacity,
    /// The declared minimum and maximum capacities disagree with the template list.
    OverlayBounds,
    /// A template descriptor size disagrees with its declared capacity.
    OverlaySize,
    /// The ELF and PVH contract version or the CPU architecture is not the supported one.
    KernelContract,
    /// The kernel image size is zero or beyond the profile bound.
    KernelSize,
    /// The initramfs layout version is not the one this compiler produces.
    InitramfsLayout,
    /// The initramfs size is zero or beyond the profile bound.
    InitramfsSize,
    /// The guest-agent size is zero or beyond the profile bound.
    GuestAgentSize,
    /// The guest-agent provenance string is empty or beyond its bound.
    GuestAgentProvenance,
    /// A guest protocol version is not the one this profile speaks.
    GuestProtocol,
    /// The kernel command line is not the fixed profile v1 line.
    CommandLine,
    /// A bound contract statement is not the pinned profile v1 statement.
    ContractStatement,
    /// Guest memory is outside the machine contract range.
    MemorySize,
    /// Guest memory is not a whole number of 4 KiB pages.
    MemoryAlignment,
    /// The vCPU count is not the supported one.
    VcpuCount,
    /// The memory-slot layout version is not the one the machine contract fixes.
    MemorySlotVersion,
    /// The launch-page layout version is not the one the guest protocol fixes.
    LaunchPageVersion,
    /// The snapshot binding is absent, present, or malformed for this resolution.
    SnapshotBinding,
    /// The repair policy version or the readiness command digest is not the fixed one.
    RepairPolicy,
    /// The writable-storage class does not name one certified overlay template.
    WritableStorage,
    /// The network policy class and its canonical digest disagree.
    NetworkPolicy,
    /// The workload probe is empty, oversized, relative, or carries a control byte.
    WorkloadProbe,
    /// The Instance time-to-live is zero or beyond the accepted maximum.
    Ttl,
    /// The declared artifact sizes cannot be summed without overflow.
    ArtifactSizeOverflow,
}

impl fmt::Display for Incompatibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PolicyVersion => "compiler policy version",
            Self::SourcePlatform => "source platform",
            Self::ZeroDigest => "reserved all-zero digest",
            Self::TreeSize => "normalized tree size",
            Self::RootUuid => "root filesystem UUID derivation",
            Self::RootFormat => "root format profile or formatter revision",
            Self::RootSize => "root image size",
            Self::OverlayProfile => "overlay derivation version or feature profile",
            Self::OverlayCapacity => "overlay template capacity",
            Self::OverlayBounds => "overlay capacity bounds",
            Self::OverlaySize => "overlay template size",
            Self::KernelContract => "kernel contract version or architecture",
            Self::KernelSize => "kernel image size",
            Self::InitramfsLayout => "initramfs layout version",
            Self::InitramfsSize => "initramfs size",
            Self::GuestAgentSize => "guest agent size",
            Self::GuestAgentProvenance => "guest agent provenance",
            Self::GuestProtocol => "guest protocol version",
            Self::CommandLine => "kernel command line",
            Self::ContractStatement => "bound contract statement",
            Self::MemorySize => "guest memory size",
            Self::MemoryAlignment => "guest memory alignment",
            Self::VcpuCount => "vCPU count",
            Self::MemorySlotVersion => "memory slot layout version",
            Self::LaunchPageVersion => "launch page layout version",
            Self::SnapshotBinding => "snapshot binding",
            Self::RepairPolicy => "repair policy or readiness command",
            Self::WritableStorage => "writable storage class",
            Self::NetworkPolicy => "network policy binding",
            Self::WorkloadProbe => "workload probe",
            Self::Ttl => "Instance time to live",
            Self::ArtifactSizeOverflow => "artifact size total",
        })
    }
}
