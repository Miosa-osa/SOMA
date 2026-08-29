//! Typed failures of the network broker.
//!
//! Every variant is redacted: no host path, descriptor number, or shell output is carried.

use std::{error::Error as StdError, fmt};

/// One typed broker failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The portable request left a security dimension unspecified or unsupported.
    InvalidIntent(IntentRejection),
    /// The operator profile is inconsistent.
    InvalidProfile(&'static str),
    /// The bounded lease pool is exhausted for this generation.
    PoolExhausted,
    /// The identifier is all zero or otherwise unusable.
    InvalidId(&'static str),
    /// The current process lacks `CAP_NET_ADMIN` or a required device.
    MissingPrivilege(&'static str),
    /// A kernel call failed at one typed step.
    Kernel {
        /// The step that failed.
        step: Step,
        /// The kernel errno.
        errno: i32,
    },
    /// A pinned userspace tool failed or is absent.
    Tool {
        /// The tool.
        tool: Tool,
        /// Its exit status when it ran.
        status: Option<i32>,
    },
    /// The ledger already binds this bundle and generation to a different operation.
    LedgerConflict,
    /// The ledger record exists but its intent or identities differ from the replay.
    ReplayMismatch,
    /// The ledger record is malformed or short.
    LedgerCorrupt,
    /// The ledger has no record for this bundle and generation.
    NotAssigned,
    /// Kernel state does not match the ledger record.
    Drift(Drift),
    /// The transfer header or descriptor set was rejected.
    Transfer(TransferRejection),
    /// A port reservation could not be taken exclusively.
    PortUnavailable,
    /// The feature is reserved by the specification but not implemented in this slice.
    Unimplemented(&'static str),
    /// The bundle is in the wrong lifecycle state for this call.
    InvalidState(&'static str),
    /// A protocol frame was malformed or too large.
    Protocol(&'static str),
}

/// Why a portable policy could not become a broker intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntentRejection {
    /// `EgressPolicy::Unspecified` is not admitted by the production broker.
    EgressUnspecified,
    /// `DnsPolicy::Unspecified` is not admitted by the production broker.
    DnsUnspecified,
    /// DNS was requested while egress is denied.
    DnsWithoutEgress,
    /// A custom resolver is not an IPv4 address in this profile slice.
    ResolverFamily,
    /// A custom resolver lies inside the protected destination set.
    ResolverProtected,
    /// Proxy profiles are not implemented in this slice.
    ProxyUnimplemented,
    /// A guest address was requested explicitly; only allocated addresses are supported.
    StaticAddress,
    /// IPv6 guest addressing is not implemented in this slice.
    Ipv6Unimplemented,
    /// A network profile selector named a profile that this broker does not serve.
    ProfileMismatch,
}

/// One kernel-facing or durable-storage step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum Step {
    Unshare,
    MountNamespace,
    OpenNamespace,
    EnterNamespace,
    OpenTun,
    TunSetIff,
    Socket,
    SetHwaddr,
    SetMtu,
    GetFlags,
    SetFlags,
    SetAddress,
    SetNetmask,
    AddRoute,
    Netlink,
    Sysctl,
    Thread,
    LedgerOpen,
    LedgerWrite,
    LedgerSync,
    LedgerRead,
    Bind,
    Unmount,
    Unlink,
    SendMsg,
    RecvMsg,
    Clock,
    ListLinks,
}

/// One pinned userspace tool the version 1 broker invokes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tool {
    /// `/usr/sbin/nft`.
    Nft,
    /// `/usr/sbin/conntrack`.
    Conntrack,
}

/// One class of divergence between the ledger and the kernel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum Drift {
    NamespaceMissing,
    TapMissing,
    VethMissing,
    HostVethMissing,
    RulesetMissing,
    HostRulesetMissing,
    ForwardingAlreadyEnabled,
    LinkAlreadyUp,
}

/// Why a descriptor transfer was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(missing_docs)]
pub enum TransferRejection {
    BadMagic,
    BadVersion,
    BadLength,
    DescriptorCount,
    ControlShort,
    ZeroBundle,
    ZeroGeneration,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIntent(reason) => write!(formatter, "invalid network intent: {reason:?}"),
            Self::InvalidProfile(reason) => write!(formatter, "invalid network profile: {reason}"),
            Self::PoolExhausted => formatter.write_str("lease pool exhausted"),
            Self::InvalidId(kind) => write!(formatter, "invalid {kind} identifier"),
            Self::MissingPrivilege(what) => write!(formatter, "missing privilege: {what}"),
            Self::Kernel { step, errno } => write!(formatter, "{step:?} failed with errno {errno}"),
            Self::Tool { tool, status } => write!(formatter, "{tool:?} failed with {status:?}"),
            Self::LedgerConflict => formatter.write_str("ledger conflict"),
            Self::ReplayMismatch => formatter.write_str("replay does not match the ledger"),
            Self::LedgerCorrupt => formatter.write_str("ledger record is corrupt"),
            Self::NotAssigned => formatter.write_str("bundle is not assigned"),
            Self::Drift(drift) => write!(formatter, "kernel drift: {drift:?}"),
            Self::Transfer(reason) => write!(formatter, "transfer rejected: {reason:?}"),
            Self::PortUnavailable => formatter.write_str("port reservation unavailable"),
            Self::Unimplemented(what) => write!(formatter, "not implemented: {what}"),
            Self::InvalidState(what) => write!(formatter, "invalid state: {what}"),
            Self::Protocol(what) => write!(formatter, "protocol error: {what}"),
        }
    }
}

impl StdError for Error {}

impl Error {
    /// Builds a kernel failure from the last OS error.
    #[must_use]
    pub fn kernel(step: Step) -> Self {
        Self::Kernel {
            step,
            errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
        }
    }

    /// Builds a kernel failure from one `std::io::Error`.
    #[must_use]
    pub fn io(step: Step, error: &std::io::Error) -> Self {
        Self::Kernel {
            step,
            errno: error.raw_os_error().unwrap_or(0),
        }
    }
}
