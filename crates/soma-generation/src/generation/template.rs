use soma::{MachineShape, NetworkPolicy, OciDigest, OciImage, OciPlatform};
use soma_kvm::DeviceSet;

use super::{
    artifacts::Sha256Digest,
    contracts,
    error::{CompileError, CompileErrorKind, CompilePhase},
};

mod network;

pub use network::{NetworkPolicyClass, network_policy_digest};

const MIB: u64 = 1024 * 1024;
const MINIMUM_MEMORY_BYTES: u64 = 128 * MIB;
const MAXIMUM_MEMORY_BYTES: u64 = 3 * 1024 * MIB;
const MINIMUM_STORAGE_BYTES: u64 = 64 * MIB;
/// Maximum accepted Instance lifetime.
pub const MAX_TTL_SECONDS: u64 = 30 * 24 * 3600;
/// Maximum bytes of an explicit workload probe command line.
pub const MAX_WORKLOAD_PROBE_BYTES: usize = 4096;

/// The selected OCI image of a Template revision: the user-facing reference plus the
/// resolved immutable identity that the compiler actually consumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateImage {
    reference: OciImage,
    manifest_digest: OciDigest,
    platform: OciPlatform,
}

impl TemplateImage {
    /// Pairs a reference with the digest and platform it resolved to.
    #[must_use]
    pub const fn new(
        reference: OciImage,
        manifest_digest: OciDigest,
        platform: OciPlatform,
    ) -> Self {
        Self {
            reference,
            manifest_digest,
            platform,
        }
    }

    /// Returns the user-facing reference, which is Template provenance rather than identity.
    #[must_use]
    pub const fn reference(&self) -> &OciImage {
        &self.reference
    }

    /// Returns the resolved OCI manifest digest bound into the Generation.
    #[must_use]
    pub const fn manifest_digest(&self) -> &OciDigest {
        &self.manifest_digest
    }

    /// Returns the resolved platform bound into the Generation.
    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }
}

/// Startup and readiness behavior of a Template revision.
///
/// Readiness always uses the fixed guest-agent probe from the repair policy; an explicit
/// workload probe is an additional bounded command line that must also succeed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupBehavior {
    workload_probe: Option<Vec<u8>>,
}

impl StartupBehavior {
    /// Uses only the fixed readiness command.
    #[must_use]
    pub const fn readiness_only() -> Self {
        Self {
            workload_probe: None,
        }
    }

    /// Adds one bounded explicit workload probe command line.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::InvalidInput`] for an empty, NUL-bearing, or oversized probe.
    pub fn with_workload_probe(probe: Vec<u8>) -> Result<Self, CompileError> {
        if probe.is_empty() || probe.len() > MAX_WORKLOAD_PROBE_BYTES || probe.contains(&0) {
            return Err(invalid());
        }
        Ok(Self {
            workload_probe: Some(probe),
        })
    }

    /// Returns the digest of the fixed readiness command.
    #[must_use]
    pub fn readiness_command_digest(&self) -> Sha256Digest {
        contracts::readiness_command_digest()
    }

    /// Returns the explicit workload probe, if any.
    #[must_use]
    pub fn workload_probe(&self) -> Option<&[u8]> {
        self.workload_probe.as_deref()
    }
}

/// Lifetime limits of Instances launched from the Generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifetimeLimits {
    ttl_seconds: u64,
}

impl LifetimeLimits {
    /// Bounds one Instance lifetime in whole seconds.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::InvalidInput`] for zero or more than [`MAX_TTL_SECONDS`].
    pub fn new(ttl_seconds: u64) -> Result<Self, CompileError> {
        if ttl_seconds == 0 || ttl_seconds > MAX_TTL_SECONDS {
            return Err(invalid());
        }
        Ok(Self { ttl_seconds })
    }

    /// Returns the time-to-live in seconds.
    #[must_use]
    pub const fn ttl_seconds(&self) -> u64 {
        self.ttl_seconds
    }
}

/// One immutable revision of a user-authored Template.
///
/// Editing any field creates a new revision.
/// Every field except the image reference flows into the `SOMAGEN` manifest and therefore
/// into the `GenerationId`; the reference is provenance because the resolved digest and
/// platform are what the compiler consumes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateRevision {
    image: TemplateImage,
    shape: MachineShape,
    startup: StartupBehavior,
    lifetime: LifetimeLimits,
    profile_version: u16,
}

impl TemplateRevision {
    /// Assembles one revision and enforces the `x86_64` profile v1 shape bounds.
    ///
    /// The network policy intent lives inside the Machine shape capabilities, as the portable
    /// `soma` request contract already defines it.
    ///
    /// # Errors
    ///
    /// Returns [`CompileErrorKind::Unsupported`] for a platform other than `linux/amd64` or
    /// more than one vCPU, and [`CompileErrorKind::InvalidInput`] for memory outside 128 MiB
    /// through 3 GiB, memory not in 4 KiB units, or writable storage below 64 MiB or not in
    /// 4 MiB units.
    pub fn new(
        image: TemplateImage,
        shape: MachineShape,
        startup: StartupBehavior,
        lifetime: LifetimeLimits,
        profile_version: u16,
    ) -> Result<Self, CompileError> {
        if image.platform.operating_system() != "linux"
            || image.platform.architecture() != "amd64"
            || image.platform.variant().is_some()
            || shape.vcpu_count() != 1
        {
            return Err(CompileError::new(
                CompilePhase::ResolveInputs,
                CompileErrorKind::Unsupported,
            ));
        }
        let memory = shape.memory_mib().checked_mul(MIB).ok_or_else(invalid)?;
        let storage = shape.storage_mib().checked_mul(MIB).ok_or_else(invalid)?;
        // Zero writable storage is a machine with no overlay device at all, which is a smaller
        // machine rather than an undersized disk; every other value still has to name a real
        // size class the compiler can build a sterile template for.
        let storage_valid =
            storage == 0 || (storage >= MINIMUM_STORAGE_BYTES && storage.is_multiple_of(4 * MIB));
        if !(MINIMUM_MEMORY_BYTES..=MAXIMUM_MEMORY_BYTES).contains(&memory) || !storage_valid {
            return Err(invalid());
        }
        Ok(Self {
            image,
            shape,
            startup,
            lifetime,
            profile_version,
        })
    }

    /// Returns the selected image.
    #[must_use]
    pub const fn image(&self) -> &TemplateImage {
        &self.image
    }

    /// Returns the requested Machine shape.
    #[must_use]
    pub const fn shape(&self) -> &MachineShape {
        &self.shape
    }

    /// Returns the network policy intent carried by the Machine shape capabilities.
    #[must_use]
    pub const fn network_policy(&self) -> &NetworkPolicy {
        self.shape.capabilities().network_policy()
    }

    /// Returns the startup and readiness behavior.
    #[must_use]
    pub const fn startup(&self) -> &StartupBehavior {
        &self.startup
    }

    /// Returns the lifetime limits.
    #[must_use]
    pub const fn lifetime(&self) -> LifetimeLimits {
        self.lifetime
    }

    /// Returns the preparation profile version this revision targets.
    #[must_use]
    pub const fn profile_version(&self) -> u16 {
        self.profile_version
    }

    /// Returns guest memory in bytes.
    #[must_use]
    pub fn memory_bytes(&self) -> u64 {
        self.shape.memory_mib() * MIB
    }

    /// Returns the writable-storage size class in bytes; zero means no writable disk at all.
    #[must_use]
    pub fn writable_storage_bytes(&self) -> u64 {
        self.shape.storage_mib() * MIB
    }

    /// The optional devices this revision's machine will have.
    ///
    /// Both follow from what the revision already declares. A revision asking for no writable
    /// storage gets no overlay device, so its Instances never clone a private head; a revision
    /// whose network policy is the fail-closed isolated one gets no network device, because a
    /// device whose link can never come up is a cost with no capability behind it.
    #[must_use]
    pub fn device_set(&self) -> DeviceSet {
        DeviceSet::new(
            self.writable_storage_bytes() > 0,
            NetworkPolicyClass::of(self.network_policy()) != NetworkPolicyClass::Isolated,
        )
    }
}

const fn invalid() -> CompileError {
    CompileError::new(CompilePhase::ResolveInputs, CompileErrorKind::InvalidInput)
}
