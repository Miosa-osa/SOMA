//! The common module contract shared by every module kind.
//!
//! A module is convenience configuration expressed as data.
//! It declares what it owns, what it needs, and where it can run; it never carries
//! privileged VMM behavior, and no agent brand receives special treatment.

mod builtin;
pub(crate) mod digest;
mod path;
mod reference;
mod registry;
mod spec;

use std::{error::Error, fmt};

use crate::error::BoundError;

pub use path::{
    EnvironmentName, GuestPath, MAX_ENVIRONMENT_NAME_BYTES, MAX_PATH_BYTES, NameError, PathError,
};
pub use reference::{ModuleRef, ModuleRefError};
pub use registry::{MAX_REGISTRY_MODULES, ModuleRegistry};
pub use spec::{
    MAX_FIELD_NAME_BYTES, MAX_MODULE_LIST, MAX_SEALED_VALUE_BYTES, ModuleBuilder, ModuleSpec,
};

pub const MAX_MODULE_NAME_BYTES: usize = 64;
pub const MAX_DESTINATION_HOST_BYTES: usize = 253;

/// The module kinds accepted by composition version 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ModuleKind {
    Agent,
    Tools,
    Workspace,
    Network,
    Environment,
    Secrets,
    Lifecycle,
    Resources,
}

impl ModuleKind {
    pub const ALL: [Self; 8] = [
        Self::Agent,
        Self::Tools,
        Self::Workspace,
        Self::Network,
        Self::Environment,
        Self::Secrets,
        Self::Lifecycle,
        Self::Resources,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Tools => "tools",
            Self::Workspace => "workspace",
            Self::Network => "network",
            Self::Environment => "environment",
            Self::Secrets => "secrets",
            Self::Lifecycle => "lifecycle",
            Self::Resources => "resources",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Agent => 1,
            Self::Tools => 2,
            Self::Workspace => 3,
            Self::Network => 4,
            Self::Environment => 5,
            Self::Secrets => 6,
            Self::Lifecycle => 7,
            Self::Resources => 8,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Agent),
            2 => Some(Self::Tools),
            3 => Some(Self::Workspace),
            4 => Some(Self::Network),
            5 => Some(Self::Environment),
            6 => Some(Self::Secrets),
            7 => Some(Self::Lifecycle),
            8 => Some(Self::Resources),
            _ => None,
        }
    }
}

impl fmt::Display for ModuleKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One exact pinned module: kind, name, and version.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ModuleIdentity {
    kind: ModuleKind,
    name: String,
    version: u32,
}

impl ModuleIdentity {
    /// Creates one pinned identity.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError::ForbiddenCharacter`] unless the name is lowercase ASCII letters,
    /// digits, and interior hyphens within [`MAX_MODULE_NAME_BYTES`].
    pub fn new(kind: ModuleKind, name: &str, version: u32) -> Result<Self, BoundError> {
        if !reference::valid_name(name) {
            return Err(BoundError::ForbiddenCharacter {
                field: "name".to_owned(),
            });
        }
        Ok(Self::unchecked(kind, name.to_owned(), version))
    }

    pub(crate) const fn unchecked(kind: ModuleKind, name: String, version: u32) -> Self {
        Self {
            kind,
            name,
            version,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ModuleKind {
        self.kind
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

impl fmt::Display for ModuleIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "soma://{}/{}@{}",
            self.kind, self.name, self.version
        )
    }
}

/// One optional network destination a module may want to reach.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Destination {
    host: String,
    port: u16,
}

impl Destination {
    /// Parses `host:port`; the host and port are shape-checked during validation.
    ///
    /// # Errors
    ///
    /// Returns [`BoundError`] for a missing separator, empty host, oversized host, or a port
    /// outside `u16`.
    pub fn parse(value: &str) -> Result<Self, BoundError> {
        let field = || "destination".to_owned();
        let (host, port) = value
            .rsplit_once(':')
            .ok_or_else(|| BoundError::ForbiddenCharacter { field: field() })?;
        if host.is_empty() {
            return Err(BoundError::Empty { field: field() });
        }
        if host.len() > MAX_DESTINATION_HOST_BYTES {
            return Err(BoundError::TooLong {
                field: field(),
                maximum: MAX_DESTINATION_HOST_BYTES,
            });
        }
        let port = port
            .parse()
            .map_err(|_| BoundError::ForbiddenCharacter { field: field() })?;
        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// How a Backend may check that an installed module is alive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HealthProbe {
    Command {
        program: String,
        args: Vec<String>,
        timeout_seconds: u32,
    },
    Tcp {
        port: u16,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModuleError {
    Bound(BoundError),
    NoPlatform,
    ZeroSchemaVersion,
    DuplicateModule(ModuleIdentity),
    RegistryFull { maximum: usize },
}

impl fmt::Display for ModuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bound(bound) => bound.fmt(formatter),
            Self::NoPlatform => formatter.write_str("module declares no supported platform"),
            Self::ZeroSchemaVersion => formatter.write_str("module schema version must be nonzero"),
            Self::DuplicateModule(identity) => {
                write!(formatter, "module {identity} is already registered")
            }
            Self::RegistryFull { maximum } => {
                write!(formatter, "module registry holds at most {maximum} modules")
            }
        }
    }
}

impl Error for ModuleError {}

impl From<BoundError> for ModuleError {
    fn from(bound: BoundError) -> Self {
        Self::Bound(bound)
    }
}
