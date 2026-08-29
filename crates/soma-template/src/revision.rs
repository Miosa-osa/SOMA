//! `TemplateRevision`: the lock projected onto the fields the Generation compiler consumes.
//!
//! The Generation compiler on its own branch binds a `SOMAGEN` manifest and derives
//! `GenerationId` from it.
//! This view is the input contract it consumes from the preparation plane; it adds no
//! information beyond the lock and hides the fields that belong to Launch policy only.
//!
//! # Mapping to the Generation manifest
//!
//! | Revision field | CONTEXT.md meaning | Generation manifest binding |
//! |---|---|---|
//! | `lock_id` | Identity of the Template revision | Certification evidence binds the lock identity (ADR 0022) |
//! | `workload_digest`, `workload_platform`, `workload_size` | The selected OCI image of the Template | Item 2: source OCI manifest digest and effective OCI platform |
//! | `modules` | The ordered focused modules of the Template | Inputs to deterministic filesystem construction; their digests enter item 3 through the normalized tree |
//! | `vcpus`, `memory_mib` | The Machine shape requested for every Instance | Item 13: memory size and vCPU count |
//! | `writable_storage_mib` | The writable-storage capacity of the Machine shape | Item 5: overlay-template capacity class selection |
//! | `default_command` | The Workload program started once an Instance is Ready | Guest-agent launch configuration; never kernel command-line text |
//! | `environment` | Launch-time environment contract | Guest-agent launch configuration; names and literals only |
//! | `secrets` | Secret references with delivery and scope | Launch-time delivery plan; never a Generation artifact |
//! | `network` | The maximum permission envelope Launch may narrow | Backend network profile selection; not bound into immutable bytes |
//! | `lifecycle` | Idle and lifetime policy of an Instance | Backend lifecycle policy; not bound into immutable bytes |
//! | `content_digest` | The authored content the lock was derived from | Provenance retained beside the manifest |
//!
//! The kernel, initramfs, guest agent, machine and device contracts, CPU template, and
//! snapshot state are compiler-policy inputs that the Template does not select.

use soma::{OciDigest, OciPlatform};

use crate::{
    identity::LockId,
    lock::{LockedCommand, LockedEnvironment, LockedModule, LockedSecret, TemplateLock},
    schema::Lifecycle,
    validate::NetworkEnvelope,
};

/// One immutable Template revision as the Generation compiler sees it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateRevision {
    lock_id: LockId,
    content_digest: [u8; 32],
    workload_digest: OciDigest,
    workload_platform: OciPlatform,
    workload_size: u64,
    modules: Vec<LockedModule>,
    vcpus: u32,
    memory_mib: u64,
    writable_storage_mib: u64,
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
            workload_digest: lock.image().digest().clone(),
            workload_platform: lock.image().platform().clone(),
            workload_size: lock.image().size(),
            modules: lock.modules().to_vec(),
            vcpus: lock.resources().vcpus,
            memory_mib: lock.resources().memory_mib,
            writable_storage_mib: lock.resources().writable_storage_mib,
            default_command: lock.command().clone(),
            environment: lock.environment().to_vec(),
            secrets: lock.secrets().to_vec(),
            network: lock.network().clone(),
            lifecycle: *lock.lifecycle(),
        }
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
    pub const fn workload_digest(&self) -> &OciDigest {
        &self.workload_digest
    }

    #[must_use]
    pub const fn workload_platform(&self) -> &OciPlatform {
        &self.workload_platform
    }

    #[must_use]
    pub const fn workload_size(&self) -> u64 {
        self.workload_size
    }

    #[must_use]
    pub fn modules(&self) -> &[LockedModule] {
        &self.modules
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
