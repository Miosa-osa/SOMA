//! The `soma.template/v1alpha1` document model.
//!
//! The document is authored as TOML.
//! An unsupported schema version is rejected before any other rule, and unknown fields are
//! rejected during parsing, before any validation rule runs, so a field name typo can never
//! silently disable a policy; within one table a missing or mistyped required field is
//! reported before an unknown key.
//!
//! Field notes that go beyond the minimum example in the template system design:
//!
//! - `[command]` may be omitted; composition then requires exactly one module default.
//! - `[command] args` may be omitted and defaults to no arguments.
//! - `[command] user` is an optional POSIX user name; version 1 defaults to `root`.
//! - `[network]` may be omitted and defaults to denied egress and denied ingress.
//! - `[resources]` may be omitted, and so may any field in it; the defaults are the same
//!   `MachineShape` constants the command line already defaults to, so a Template and a
//!   `soma run` invocation cannot disagree about what an unstated Machine shape means.
//! - `[lifecycle]` may be omitted, and so may any field in it; see the default constants.
//! - `[[secrets]] scope` is required for `file` and `egress-proxy` delivery, and defaults to
//!   the secret name for `environment` delivery; `mode` applies only to `file` delivery.
//! - `[[environment]]` entries carry either a literal `value` or `required = true`.
//! - `[resources]` carries vCPUs, memory, and writable storage only; the process count, open
//!   file, and output limits of the design are not yet accepted.
//! - `[resources] writable_storage_mib = 0` is a sandbox with no writable disk at all. Its
//!   Generation builds no overlay device, its guest mounts the immutable root read-only, and no
//!   private disk head is cloned on the launch path. Zero vCPUs or zero memory stay rejected,
//!   because neither describes a machine.
//! - `[network]` carries egress, domain and CIDR destinations, and ingress only; DNS
//!   behavior, protocols, and ports are not yet accepted.
//!
//! A document has no content digest of its own: the digest bound into a lock is computed
//! from the composed selection, so the authored spelling of one selection never splits it.

mod choice;
mod command;
mod parse;
mod reader;

use soma::{MachineShape, OciImage, OciPlatform};

use crate::module::ModuleRef;

pub use choice::{EgressIntent, IdleAction, IngressIntent, SecretDelivery};
pub use command::Command;
pub use parse::parse_template;

pub const SCHEMA: &str = "soma.template/v1alpha1";
pub const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
pub const MAX_STRING_BYTES: usize = 4096;
pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_MODULES: usize = 64;
pub const MAX_ARGUMENTS: usize = 64;
pub const MAX_ENVIRONMENT: usize = 256;
pub const MAX_SECRETS: usize = 64;
pub const MAX_DOMAINS: usize = 256;
pub const MAX_CIDRS: usize = 256;
pub const DEFAULT_WORKING_DIRECTORY: &str = "/";
pub const DEFAULT_USER: &str = "root";
/// Owner-read-only, the mode applied to a file-delivered secret without an explicit `mode`.
pub const DEFAULT_SECRET_FILE_MODE: u32 = 0o400;
/// Long enough that no interactive round trip trips it, short enough that a sandbox nobody is
/// talking to is reclaimed in minutes rather than hours.
pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u64 = 300;
/// One hour: an abandoned sandbox must have an end, and an hour is the longest run a caller
/// can forget about without noticing the bill or the machine.
pub const DEFAULT_MAXIMUM_LIFETIME_SECONDS: u64 = 3_600;
/// Destroying an idle sandbox leaves nothing behind, so it is the only choice that is safe to
/// apply to a Template whose author never thought about idleness. `stop` and `checkpoint` both
/// retain guest state and are decisions an author has to make on purpose.
pub const DEFAULT_ON_IDLE: IdleAction = IdleAction::Destroy;

/// One parsed Template document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Template {
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) workload: Workload,
    pub(crate) modules: Vec<ModuleRef>,
    pub(crate) command: Option<Command>,
    pub(crate) resources: Resources,
    pub(crate) network: Network,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) environment: Vec<EnvironmentEntry>,
    pub(crate) secrets: Vec<SecretReference>,
}

impl Template {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    #[must_use]
    pub const fn workload(&self) -> &Workload {
        &self.workload
    }

    #[must_use]
    pub fn modules(&self) -> &[ModuleRef] {
        &self.modules
    }

    #[must_use]
    pub const fn command(&self) -> Option<&Command> {
        self.command.as_ref()
    }

    #[must_use]
    pub const fn resources(&self) -> &Resources {
        &self.resources
    }

    #[must_use]
    pub const fn network(&self) -> &Network {
        &self.network
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &Lifecycle {
        &self.lifecycle
    }

    #[must_use]
    pub fn environment(&self) -> &[EnvironmentEntry] {
        &self.environment
    }

    #[must_use]
    pub fn secrets(&self) -> &[SecretReference] {
        &self.secrets
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workload {
    pub(crate) image: OciImage,
    pub(crate) platform: OciPlatform,
}

impl Workload {
    #[must_use]
    pub const fn image(&self) -> &OciImage {
        &self.image
    }

    #[must_use]
    pub const fn platform(&self) -> &OciPlatform {
        &self.platform
    }
}

/// The requested Machine shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Resources {
    pub vcpus: u32,
    pub memory_mib: u64,
    pub writable_storage_mib: u64,
}

impl Default for Resources {
    /// The values are read from `MachineShape` rather than restated here, because the command
    /// line already defaults its `--vcpus`, `--memory-mib`, and `--storage-mib` flags to those
    /// same constants. Restating them would let a Template and a `soma run` drift apart into
    /// two different meanings of "default", which is worse than the verbosity they replace.
    fn default() -> Self {
        Self {
            vcpus: u32::from(MachineShape::DEFAULT_VCPU_COUNT),
            memory_mib: MachineShape::DEFAULT_MEMORY_MIB,
            writable_storage_mib: MachineShape::DEFAULT_STORAGE_MIB,
        }
    }
}

/// The authored network intent before normalization into an envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Network {
    pub egress: EgressIntent,
    pub allow_domains: Vec<String>,
    pub allow_cidrs: Vec<String>,
    pub ingress: IngressIntent,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            egress: EgressIntent::Deny,
            allow_domains: Vec::new(),
            allow_cidrs: Vec::new(),
            ingress: IngressIntent::Deny,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Lifecycle {
    pub idle_timeout_seconds: u64,
    pub maximum_lifetime_seconds: u64,
    pub on_idle: IdleAction,
}

impl Default for Lifecycle {
    /// Nothing outside this crate has an opinion about lifecycle yet, so unlike the Machine
    /// shape there is no existing constant to reuse; the three constants above are the single
    /// definition, and the reasoning for each sits with it.
    fn default() -> Self {
        Self {
            idle_timeout_seconds: DEFAULT_IDLE_TIMEOUT_SECONDS,
            maximum_lifetime_seconds: DEFAULT_MAXIMUM_LIFETIME_SECONDS,
            on_idle: DEFAULT_ON_IDLE,
        }
    }
}

/// One declared environment slot: a literal value or a name that Launch must supply.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvironmentEntry {
    pub name: String,
    pub value: Option<String>,
}

/// A reference to a secret held outside the Template; never a value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SecretReference {
    pub name: String,
    pub source: String,
    pub delivery: SecretDelivery,
    pub scope: Option<String>,
    pub mode: Option<u32>,
}
