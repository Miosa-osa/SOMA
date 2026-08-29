//! The typed records bound into a Template Lock.

use crate::{module::ModuleIdentity, schema::SecretDelivery};

/// One module bound into the lock by identity, contract schema, and content digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedModule {
    pub(crate) identity: ModuleIdentity,
    pub(crate) schema_version: u16,
    pub(crate) digest: [u8; 32],
}

impl LockedModule {
    #[must_use]
    pub const fn identity(&self) -> &ModuleIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// The effective default command with defaults applied.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) working_directory: String,
    pub(crate) user: String,
}

impl LockedCommand {
    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    #[must_use]
    pub fn args(&self) -> &[String] {
        &self.args
    }

    #[must_use]
    pub fn working_directory(&self) -> &str {
        &self.working_directory
    }

    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }
}

/// One effective environment slot: a Template literal, a Launch-required name, or a value
/// sealed by a module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedEnvironment {
    pub(crate) name: String,
    pub(crate) value: Option<String>,
    pub(crate) sealed_by: Option<ModuleIdentity>,
}

impl LockedEnvironment {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The literal value, or `None` when Launch must supply it.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }

    #[must_use]
    pub const fn sealed_by(&self) -> Option<&ModuleIdentity> {
        self.sealed_by.as_ref()
    }
}

/// One secret reference with its delivery and destination scope; never a value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedSecret {
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) delivery: SecretDelivery,
    pub(crate) scope: String,
    pub(crate) mode: Option<u32>,
}

impl LockedSecret {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub const fn delivery(&self) -> SecretDelivery {
        self.delivery
    }

    /// The environment name, guest file path, or egress destination for the delivery.
    #[must_use]
    pub fn scope(&self) -> &str {
        &self.scope
    }

    #[must_use]
    pub const fn mode(&self) -> Option<u32> {
        self.mode
    }
}
