//! The typed rejection vocabulary required by the template system specification.
//!
//! Every rejection names the module that caused it when a module is involved and the exact
//! field responsible, so a user can correct the Template without reading resolver internals.

mod display;

use std::fmt;

use crate::module::ModuleIdentity;

/// The ten rejection classes listed under "Required validation" in the template system design.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum RejectionClass {
    /// Mutable image input that cannot be resolved to an exact digest.
    UnresolvableImage,
    /// Unsupported architecture or incompatible agent module.
    IncompatibleModule,
    /// Duplicate exclusive ownership or conflicting default commands.
    ExclusiveConflict,
    /// A secret literal in a committed Template.
    SecretLiteral,
    /// A secret reference without a declared delivery and destination scope.
    SecretWithoutScope,
    /// Network permissions wider than an organization's policy ceiling.
    NetworkExceedsCeiling,
    /// An agent command whose executable is absent from the resolved filesystem.
    ExecutableAbsent,
    /// An invalid working directory, user, ownership mode, port, timeout, or resource dimension.
    InvalidValue,
    /// A lifecycle action unsupported by the selected Backend.
    UnsupportedLifecycleAction,
    /// A module graph containing a cycle or an unpinned transitive input.
    ModuleGraph,
}

impl RejectionClass {
    pub const ALL: [Self; 10] = [
        Self::UnresolvableImage,
        Self::IncompatibleModule,
        Self::ExclusiveConflict,
        Self::SecretLiteral,
        Self::SecretWithoutScope,
        Self::NetworkExceedsCeiling,
        Self::ExecutableAbsent,
        Self::InvalidValue,
        Self::UnsupportedLifecycleAction,
        Self::ModuleGraph,
    ];
}

/// Why one value failed the shape rules of its field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidReason {
    Zero,
    ExceedsMaximum { maximum: u64 },
    Empty,
    NotAbsolutePath,
    NotNormalizedPath,
    InvalidUser,
    InvalidMode,
    InvalidPort,
    InvalidTimeout,
    TimeoutOrdering,
    InvalidDomain,
    InvalidCidr,
    ContradictoryEgress,
    EmptyAllowlist,
    Duplicate,
    DestinationNotAllowed,
    ForbiddenCharacter,
}

/// One rejection with the module and field responsible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Rejection {
    UnresolvableImage {
        field: String,
        reference: String,
        platform: String,
    },
    UnsupportedPlatform {
        module: Option<ModuleIdentity>,
        field: String,
        platform: String,
    },
    MissingRequiredEnvironment {
        module: ModuleIdentity,
        field: String,
        name: String,
    },
    DuplicateExclusiveOwnership {
        first: ModuleIdentity,
        second: ModuleIdentity,
        field: String,
        owned: String,
    },
    ConflictingDefaultCommands {
        first: ModuleIdentity,
        second: ModuleIdentity,
        field: String,
    },
    ConflictingSealedEnvironment {
        module: ModuleIdentity,
        conflicting_module: Option<ModuleIdentity>,
        field: String,
        name: String,
    },
    MissingDefaultCommand {
        field: String,
    },
    SecretLiteral {
        field: String,
        name: String,
    },
    SecretWithoutScope {
        field: String,
        name: String,
    },
    NetworkExceedsCeiling {
        field: String,
        requested: String,
        ceiling: String,
    },
    ExecutableAbsent {
        field: String,
        program: String,
    },
    InvalidValue {
        module: Option<ModuleIdentity>,
        field: String,
        reason: InvalidReason,
    },
    UnsupportedLifecycleAction {
        field: String,
        action: String,
    },
    ModuleCycle {
        module: ModuleIdentity,
        field: String,
        cycle: Vec<ModuleIdentity>,
    },
    UnpinnedInput {
        module: Option<ModuleIdentity>,
        field: String,
        reference: String,
    },
    UnknownModule {
        module: Option<ModuleIdentity>,
        field: String,
        reference: String,
    },
    DuplicateModule {
        field: String,
        reference: String,
    },
}

impl Rejection {
    /// The specification class this rejection belongs to.
    #[must_use]
    pub const fn class(&self) -> RejectionClass {
        match self {
            Self::UnresolvableImage { .. } => RejectionClass::UnresolvableImage,
            Self::UnsupportedPlatform { .. } | Self::MissingRequiredEnvironment { .. } => {
                RejectionClass::IncompatibleModule
            }
            Self::DuplicateExclusiveOwnership { .. }
            | Self::ConflictingDefaultCommands { .. }
            | Self::ConflictingSealedEnvironment { .. }
            | Self::MissingDefaultCommand { .. } => RejectionClass::ExclusiveConflict,
            Self::SecretLiteral { .. } => RejectionClass::SecretLiteral,
            Self::SecretWithoutScope { .. } => RejectionClass::SecretWithoutScope,
            Self::NetworkExceedsCeiling { .. } => RejectionClass::NetworkExceedsCeiling,
            Self::ExecutableAbsent { .. } => RejectionClass::ExecutableAbsent,
            Self::InvalidValue { .. } => RejectionClass::InvalidValue,
            Self::UnsupportedLifecycleAction { .. } => RejectionClass::UnsupportedLifecycleAction,
            Self::ModuleCycle { .. }
            | Self::UnpinnedInput { .. }
            | Self::UnknownModule { .. }
            | Self::DuplicateModule { .. } => RejectionClass::ModuleGraph,
        }
    }

    /// The exact field responsible, as a dotted path with list indexes.
    #[must_use]
    pub fn field(&self) -> &str {
        match self {
            Self::UnresolvableImage { field, .. }
            | Self::UnsupportedPlatform { field, .. }
            | Self::MissingRequiredEnvironment { field, .. }
            | Self::DuplicateExclusiveOwnership { field, .. }
            | Self::ConflictingDefaultCommands { field, .. }
            | Self::ConflictingSealedEnvironment { field, .. }
            | Self::MissingDefaultCommand { field }
            | Self::SecretLiteral { field, .. }
            | Self::SecretWithoutScope { field, .. }
            | Self::NetworkExceedsCeiling { field, .. }
            | Self::ExecutableAbsent { field, .. }
            | Self::InvalidValue { field, .. }
            | Self::UnsupportedLifecycleAction { field, .. }
            | Self::ModuleCycle { field, .. }
            | Self::UnpinnedInput { field, .. }
            | Self::UnknownModule { field, .. }
            | Self::DuplicateModule { field, .. } => field,
        }
    }

    /// The module responsible, when the field belongs to a module rather than the Template.
    #[must_use]
    pub const fn module(&self) -> Option<&ModuleIdentity> {
        match self {
            Self::UnsupportedPlatform { module, .. }
            | Self::InvalidValue { module, .. }
            | Self::UnpinnedInput { module, .. }
            | Self::UnknownModule { module, .. } => module.as_ref(),
            Self::MissingRequiredEnvironment { module, .. }
            | Self::ConflictingSealedEnvironment { module, .. }
            | Self::ModuleCycle { module, .. } => Some(module),
            Self::DuplicateExclusiveOwnership { second, .. }
            | Self::ConflictingDefaultCommands { second, .. } => Some(second),
            Self::UnresolvableImage { .. }
            | Self::MissingDefaultCommand { .. }
            | Self::SecretLiteral { .. }
            | Self::SecretWithoutScope { .. }
            | Self::NetworkExceedsCeiling { .. }
            | Self::ExecutableAbsent { .. }
            | Self::UnsupportedLifecycleAction { .. }
            | Self::DuplicateModule { .. } => None,
        }
    }
}

impl fmt::Display for Rejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        display::rejection(self, formatter)
    }
}
