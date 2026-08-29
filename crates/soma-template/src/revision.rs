//! `TemplateRevision`: the lock projected onto the input contract of the Generation compiler.
//!
//! `soma_generation::generation::template::TemplateRevision` is assembled from an image, a
//! Machine shape, startup behavior, lifetime limits, and a compiler profile version.
//! This view derives every one of those inputs that the Template selects from the lock, keeps
//! the lock-only fields that later slices consume, and adds no information of its own.
//!
//! # Mapping onto the compiler's `TemplateRevision`
//!
//! | Compiler field | This view | Lock source |
//! |---|---|---|
//! | `image.reference` | `image().reference()` after [`TemplateRevision::with_provenance`] | none: the mutable reference is provenance and never enters the lock |
//! | `image.manifest_digest` | `image().manifest_digest()` | workload digest, layout row 6 |
//! | `image.platform` | `image().platform()` | workload platform, layout row 6 |
//! | `shape.vcpu_count` | `shape()?.vcpu_count()` | resources vCPUs, layout row 9 |
//! | `shape.memory_mib` | `shape()?.memory_mib()` | resources memory, layout row 9 |
//! | `shape.storage_mib` | `shape()?.storage_mib()` | resources writable storage, layout row 9 |
//! | `shape.capabilities.network` | `shape()?.capabilities().network_policy()` | network envelope, layout row 10; exact for a fully denied envelope and for unrestricted egress with denied ingress, otherwise fails closed until the policy compiler exists |
//! | `startup.workload_probe` | none, readiness only | module digests in layout row 7 bind each health probe; the build plan turns them into a workload probe |
//! | `lifetime.ttl_seconds` | `ttl_seconds()` | lifecycle maximum lifetime, layout row 11 |
//! | `profile_version` | none, a Generation builder input | `policy_version()` in layout row 4 is the composition rule set, not the compiler profile |
//!
//! The remaining lock fields are not consumed by the compiler's revision and are carried for
//! later slices: `modules()` and `default_command()` feed the deterministic build plan and the
//! agent command contract, `environment()` and `secrets()` feed launch-input delivery,
//! `network()` and `lifecycle()` feed policy narrowing and Backend lifecycle selection, and
//! `content_digest()` plus `lock_id()` are provenance that certification binds.

mod network;

use std::{error::Error, fmt};

use soma::{Capabilities, MachineShape, OciDigest, OciImage, OciPlatform};

use crate::{
    identity::LockId,
    lock::{LockedCommand, LockedEnvironment, LockedModule, LockedSecret, TemplateLock},
    schema::{Lifecycle, Template},
    validate::NetworkEnvelope,
};

/// Why a lock could not be projected onto the portable request contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionError {
    /// The Template document is not the one the lock was derived from.
    ProvenanceMismatch,
    /// The locked resources do not form a portable Machine shape.
    InvalidShape,
    /// The portable network contract cannot express the locked envelope yet.
    UnrepresentableNetwork,
}

impl fmt::Display for RevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ProvenanceMismatch => "template content digest does not match the lock",
            Self::InvalidShape => "locked resources do not form a portable Machine shape",
            Self::UnrepresentableNetwork => {
                "locked network envelope has no portable network policy yet"
            }
        })
    }
}

impl Error for RevisionError {}

/// The selected image as the compiler's `TemplateImage` sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionImage {
    reference: Option<OciImage>,
    manifest_digest: OciDigest,
    platform: OciPlatform,
}

impl RevisionImage {
    /// The authored reference, present only when provenance was attached.
    #[must_use]
    pub const fn reference(&self) -> Option<&OciImage> {
        self.reference.as_ref()
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &OciDigest {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }
}

/// One immutable Template revision as the Generation compiler sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateRevision {
    lock_id: LockId,
    content_digest: [u8; 32],
    policy_version: u16,
    image: RevisionImage,
    vcpus: u32,
    memory_mib: u64,
    writable_storage_mib: u64,
    modules: Vec<LockedModule>,
    default_command: LockedCommand,
    environment: Vec<LockedEnvironment>,
    secrets: Vec<LockedSecret>,
    network: NetworkEnvelope,
    lifecycle: Lifecycle,
}

impl TemplateRevision {
    /// Projects one lock; the identity is computed from the lock's canonical bytes.
    #[must_use]
    pub fn from_lock(lock: &TemplateLock) -> Self {
        Self {
            lock_id: lock.id(),
            content_digest: *lock.content_digest(),
            policy_version: lock.policy_version(),
            image: RevisionImage {
                reference: None,
                manifest_digest: lock.image().digest().clone(),
                platform: lock.image().platform().clone(),
            },
            vcpus: lock.resources().vcpus,
            memory_mib: lock.resources().memory_mib,
            writable_storage_mib: lock.resources().writable_storage_mib,
            modules: lock.modules().to_vec(),
            default_command: lock.command().clone(),
            environment: lock.environment().to_vec(),
            secrets: lock.secrets().to_vec(),
            network: lock.network().clone(),
            lifecycle: *lock.lifecycle(),
        }
    }

    /// Attaches the authored image reference from the document the lock was derived from.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionError::ProvenanceMismatch`] when the document's content digest is
    /// not the digest bound into the lock.
    pub fn with_provenance(mut self, template: &Template) -> Result<Self, RevisionError> {
        if template.content_digest() != self.content_digest {
            return Err(RevisionError::ProvenanceMismatch);
        }
        self.image.reference = Some(template.workload().image().clone());
        Ok(self)
    }

    #[must_use]
    pub const fn lock_id(&self) -> LockId {
        self.lock_id
    }

    #[must_use]
    pub const fn content_digest(&self) -> &[u8; 32] {
        &self.content_digest
    }

    #[must_use]
    pub const fn policy_version(&self) -> u16 {
        self.policy_version
    }

    #[must_use]
    pub const fn image(&self) -> &RevisionImage {
        &self.image
    }

    /// The portable Machine shape with the network policy the envelope maps onto.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionError::InvalidShape`] for a zero dimension or more vCPUs than the
    /// portable contract carries, and [`RevisionError::UnrepresentableNetwork`] when the
    /// envelope has no exact portable policy.
    pub fn shape(&self) -> Result<MachineShape, RevisionError> {
        let vcpus = u16::try_from(self.vcpus).map_err(|_| RevisionError::InvalidShape)?;
        let shape = MachineShape::new(vcpus, self.memory_mib, self.writable_storage_mib)
            .map_err(|_| RevisionError::InvalidShape)?;
        let policy = network::policy(&self.network)?;
        Ok(shape.with_capabilities(Capabilities::isolated().with_network_policy(policy)))
    }

    #[must_use]
    pub const fn vcpus(&self) -> u32 {
        self.vcpus
    }

    #[must_use]
    pub const fn memory_mib(&self) -> u64 {
        self.memory_mib
    }

    #[must_use]
    pub const fn writable_storage_mib(&self) -> u64 {
        self.writable_storage_mib
    }

    /// The absolute Instance lifetime, which is the compiler's `LifetimeLimits`.
    #[must_use]
    pub const fn ttl_seconds(&self) -> u64 {
        self.lifecycle.maximum_lifetime_seconds
    }

    #[must_use]
    pub fn modules(&self) -> &[LockedModule] {
        &self.modules
    }

    #[must_use]
    pub const fn default_command(&self) -> &LockedCommand {
        &self.default_command
    }

    #[must_use]
    pub fn environment(&self) -> &[LockedEnvironment] {
        &self.environment
    }

    #[must_use]
    pub fn secrets(&self) -> &[LockedSecret] {
        &self.secrets
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkEnvelope {
        &self.network
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
    }
}
