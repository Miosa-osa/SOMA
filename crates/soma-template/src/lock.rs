//! The canonical Template Lock.
//!
//! A lock records exactly which inputs resolution selected.
//! Its bytes are a fixed-order big-endian encoding with explicit presence bytes and
//! length-prefixed bounded strings, and it contains no map with implementation-dependent
//! order, so equal inputs always produce equal bytes and one [`LockId`].
//!
//! # Layout (`SOMALOCK` version 1)
//!
//! | Order | Field | Encoding |
//! |---|---|---|
//! | 1 | magic | 8 bytes `SOMALOCK` |
//! | 2 | lock schema version | `u16` = 1 |
//! | 3 | template schema | string |
//! | 4 | policy version | `u16`, the composition and validation rule set |
//! | 5 | content digest | 32 bytes: SHA-256 of the composed selection (platform, ordered module identities, effective command, resources, normalized network, lifecycle, effective environment, secret references) |
//! | 6 | workload | 32-byte OCI manifest digest, `u64` size, platform (os, arch, presence + variant) |
//! | 7 | modules | count; each: kind `u8`, name, `u32` version, `u16` schema, 32-byte digest |
//! | 8 | command | program, args list, working directory, user |
//! | 9 | resources | `u32` vCPUs, `u64` memory MiB, `u64` writable storage MiB |
//! | 10 | network | egress `u8`; for allowlist a sorted domain list and a sorted canonical CIDR list; ingress `u8` |
//! | 11 | lifecycle | `u64` idle seconds, `u64` maximum seconds, on-idle `u8` |
//! | 12 | environment | count; each, sorted by unique name: name, presence + value, presence + sealing module identity |
//! | 13 | secrets | count; each, sorted by unique name: name, source, delivery `u8`, scope, presence + `u32` mode |
//! | 14 | policy ceiling | egress `u8`, presence + domain list, presence + CIDR list, ingress `u8` |
//! | 15 | Backend capabilities | platform list, idle-action mask `u8`, `u32`, `u64`, `u64` limits |
//!
//! A string is a `u32` big-endian length followed by UTF-8 bytes; a list is a `u16` count
//! followed by its elements; a presence byte is exactly 0 or 1.
//! Environment entries and secrets are name-unique sets, so they are encoded sorted by name
//! and the decoder rejects any other order, as it does for destination lists.
//! Discriminants: egress deny 0, allowlist 1, unrestricted 2; ingress deny 0, unrestricted 1;
//! on-idle destroy 0, stop 1, checkpoint 2; delivery environment 0, file 1, egress-proxy 2;
//! module kinds agent 1 through resources 8 in declaration order.
//!
//! # Excluded from the lock
//!
//! `name`, `description`, the mutable `workload.image` reference text, and every TOML
//! formatting detail are non-content and never enter the bytes.
//! Secret values never exist in this crate; only references, delivery, and scope are bound.

mod decode;
mod encode;
mod fields;

use crate::{
    compose::Composition,
    error::LockError,
    identity::LockId,
    resolve::ResolvedImage,
    schema::{Lifecycle, Resources, Template},
    validate::{BackendCapabilities, NetworkEnvelope, PolicyCeiling, Validated},
};

pub use fields::{LockedCommand, LockedEnvironment, LockedModule, LockedSecret};

pub const LOCK_MAGIC: &[u8; 8] = b"SOMALOCK";
pub const LOCK_SCHEMA_VERSION: u16 = 1;
/// The version of the composition and validation rules that produced the lock.
pub const POLICY_VERSION: u16 = 1;
pub const MAX_LOCK_MODULES: usize = crate::module::MAX_REGISTRY_MODULES;
pub const MAX_LOCK_ENVIRONMENT: usize = 4096;

/// The resolved, validated, canonical result of one Template.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TemplateLock {
    pub(crate) template_schema: String,
    pub(crate) policy_version: u16,
    pub(crate) content_digest: [u8; 32],
    pub(crate) image: ResolvedImage,
    pub(crate) modules: Vec<LockedModule>,
    pub(crate) command: LockedCommand,
    pub(crate) resources: Resources,
    pub(crate) network: NetworkEnvelope,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) environment: Vec<LockedEnvironment>,
    pub(crate) secrets: Vec<LockedSecret>,
    pub(crate) ceiling: PolicyCeiling,
    pub(crate) backend: BackendCapabilities,
}

impl TemplateLock {
    pub(crate) fn assemble(
        template: &Template,
        composition: &Composition<'_>,
        image: ResolvedImage,
        validated: Validated,
        ceiling: &PolicyCeiling,
        backend: &BackendCapabilities,
    ) -> Self {
        let modules = composition
            .modules
            .iter()
            .map(|module| LockedModule {
                identity: module.identity().clone(),
                schema_version: module.schema_version(),
                digest: module.digest(),
            })
            .collect();
        Self {
            template_schema: crate::schema::SCHEMA.to_owned(),
            policy_version: POLICY_VERSION,
            content_digest: composition.content_digest(template),
            image,
            modules,
            command: validated.command,
            resources: *template.resources(),
            network: validated.network,
            lifecycle: *template.lifecycle(),
            environment: validated.environment,
            secrets: validated.secrets,
            ceiling: ceiling.clone(),
            backend: backend.clone(),
        }
    }

    /// The canonical bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        encode::encode(self)
    }

    /// Decodes canonical bytes, rejecting any malformed, unbounded, or trailing content.
    ///
    /// # Errors
    ///
    /// Returns [`LockError`] for a bad magic, unsupported version, wire violation, invalid
    /// discriminant, or a field that fails its shape rule.
    pub fn decode(bytes: &[u8]) -> Result<Self, LockError> {
        decode::decode(bytes)
    }

    /// The content identity of the canonical bytes.
    #[must_use]
    pub fn id(&self) -> LockId {
        LockId::of(&self.encode())
    }

    #[must_use]
    pub fn template_schema(&self) -> &str {
        &self.template_schema
    }

    #[must_use]
    pub const fn policy_version(&self) -> u16 {
        self.policy_version
    }

    /// The digest of the composed selection, independent of every external input.
    #[must_use]
    pub const fn content_digest(&self) -> &[u8; 32] {
        &self.content_digest
    }

    #[must_use]
    pub const fn image(&self) -> &ResolvedImage {
        &self.image
    }

    #[must_use]
    pub fn modules(&self) -> &[LockedModule] {
        &self.modules
    }

    #[must_use]
    pub const fn command(&self) -> &LockedCommand {
        &self.command
    }

    #[must_use]
    pub const fn resources(&self) -> &Resources {
        &self.resources
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkEnvelope {
        &self.network
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
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
    pub const fn ceiling(&self) -> &PolicyCeiling {
        &self.ceiling
    }

    #[must_use]
    pub const fn backend(&self) -> &BackendCapabilities {
        &self.backend
    }
}
