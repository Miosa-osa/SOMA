//! `ModuleSpec`: the complete data contract of one module version, and its builder.

use soma::OciPlatform;

use super::{
    Destination, EnvironmentName, GuestPath, HealthProbe, ModuleError, ModuleIdentity, ModuleRef,
    digest,
};
use crate::{error::BoundError, schema::Command};

pub const MAX_MODULE_LIST: usize = 64;
pub const MAX_FIELD_NAME_BYTES: usize = 128;
pub const MAX_SEALED_VALUE_BYTES: usize = 4096;

/// The complete data contract of one module version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModuleSpec {
    identity: ModuleIdentity,
    schema_version: u16,
    requires: Vec<ModuleRef>,
    exclusive_fields: Vec<String>,
    owned_paths: Vec<GuestPath>,
    executables: Vec<GuestPath>,
    required_environment: Vec<EnvironmentName>,
    secret_environment: Vec<EnvironmentName>,
    sealed_environment: Vec<(EnvironmentName, String)>,
    destinations: Vec<Destination>,
    health_probe: Option<HealthProbe>,
    platforms: Vec<OciPlatform>,
    default_command: Option<Command>,
}

impl ModuleSpec {
    #[must_use]
    pub fn builder(identity: ModuleIdentity, schema_version: u16) -> ModuleBuilder {
        ModuleBuilder {
            spec: Self {
                identity,
                schema_version,
                requires: Vec::new(),
                exclusive_fields: Vec::new(),
                owned_paths: Vec::new(),
                executables: Vec::new(),
                required_environment: Vec::new(),
                secret_environment: Vec::new(),
                sealed_environment: Vec::new(),
                destinations: Vec::new(),
                health_probe: None,
                platforms: Vec::new(),
                default_command: None,
            },
        }
    }

    #[must_use]
    pub const fn identity(&self) -> &ModuleIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Transitive inputs that must be composed before this module.
    #[must_use]
    pub fn requires(&self) -> &[ModuleRef] {
        &self.requires
    }

    /// Named fields only one module in a composition may claim.
    #[must_use]
    pub fn exclusive_fields(&self) -> &[String] {
        &self.exclusive_fields
    }

    /// Guest paths this module owns exclusively.
    #[must_use]
    pub fn owned_paths(&self) -> &[GuestPath] {
        &self.owned_paths
    }

    /// Executables this module makes available in the guest.
    #[must_use]
    pub fn executables(&self) -> &[GuestPath] {
        &self.executables
    }

    /// Environment names that must be provided by the Template or a secret.
    #[must_use]
    pub fn required_environment(&self) -> &[EnvironmentName] {
        &self.required_environment
    }

    /// Environment names that must never be committed as literals.
    #[must_use]
    pub fn secret_environment(&self) -> &[EnvironmentName] {
        &self.secret_environment
    }

    /// Environment values fixed by this module.
    #[must_use]
    pub fn sealed_environment(&self) -> &[(EnvironmentName, String)] {
        &self.sealed_environment
    }

    #[must_use]
    pub fn destinations(&self) -> &[Destination] {
        &self.destinations
    }

    #[must_use]
    pub const fn health_probe(&self) -> Option<&HealthProbe> {
        self.health_probe.as_ref()
    }

    #[must_use]
    pub fn platforms(&self) -> &[OciPlatform] {
        &self.platforms
    }

    #[must_use]
    pub const fn default_command(&self) -> Option<&Command> {
        self.default_command.as_ref()
    }

    /// The content digest of this module contract.
    #[must_use]
    pub fn digest(&self) -> [u8; 32] {
        digest::digest(self)
    }
}

/// Accumulates module data and checks every bound once at [`ModuleBuilder::build`].
#[derive(Clone, Debug)]
pub struct ModuleBuilder {
    spec: ModuleSpec,
}

impl ModuleBuilder {
    #[must_use]
    pub fn requires(mut self, reference: ModuleRef) -> Self {
        self.spec.requires.push(reference);
        self
    }

    #[must_use]
    pub fn exclusive_field(mut self, field: &str) -> Self {
        self.spec.exclusive_fields.push(field.to_owned());
        self
    }

    #[must_use]
    pub fn owned_path(mut self, path: GuestPath) -> Self {
        self.spec.owned_paths.push(path);
        self
    }

    #[must_use]
    pub fn executable(mut self, path: GuestPath) -> Self {
        self.spec.executables.push(path);
        self
    }

    #[must_use]
    pub fn required_environment(mut self, name: EnvironmentName) -> Self {
        self.spec.required_environment.push(name);
        self
    }

    #[must_use]
    pub fn secret_environment(mut self, name: EnvironmentName) -> Self {
        self.spec.secret_environment.push(name);
        self
    }

    #[must_use]
    pub fn sealed_environment(mut self, name: EnvironmentName, value: &str) -> Self {
        self.spec.sealed_environment.push((name, value.to_owned()));
        self
    }

    #[must_use]
    pub fn destination(mut self, destination: Destination) -> Self {
        self.spec.destinations.push(destination);
        self
    }

    #[must_use]
    pub fn health_probe(mut self, probe: HealthProbe) -> Self {
        self.spec.health_probe = Some(probe);
        self
    }

    #[must_use]
    pub fn platform(mut self, platform: OciPlatform) -> Self {
        self.spec.platforms.push(platform);
        self
    }

    #[must_use]
    pub fn default_command(mut self, command: Command) -> Self {
        self.spec.default_command = Some(command);
        self
    }

    /// Checks every list and string bound.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError`] for a zero schema version, no platform, an oversized list,
    /// or an empty or oversized exclusive field name or sealed value.
    pub fn build(self) -> Result<ModuleSpec, ModuleError> {
        let spec = self.spec;
        if spec.schema_version == 0 {
            return Err(ModuleError::ZeroSchemaVersion);
        }
        if spec.platforms.is_empty() {
            return Err(ModuleError::NoPlatform);
        }
        let lists = [
            ("requires", spec.requires.len()),
            ("exclusive_fields", spec.exclusive_fields.len()),
            ("owned_paths", spec.owned_paths.len()),
            ("executables", spec.executables.len()),
            ("required_environment", spec.required_environment.len()),
            ("secret_environment", spec.secret_environment.len()),
            ("sealed_environment", spec.sealed_environment.len()),
            ("destinations", spec.destinations.len()),
            ("platforms", spec.platforms.len()),
        ];
        for (field, length) in lists {
            if length > MAX_MODULE_LIST {
                return Err(BoundError::TooMany {
                    field: field.to_owned(),
                    maximum: MAX_MODULE_LIST,
                }
                .into());
            }
        }
        for field in &spec.exclusive_fields {
            bounded_text("exclusive_fields", field, MAX_FIELD_NAME_BYTES)?;
        }
        for (_, value) in &spec.sealed_environment {
            if value.len() > MAX_SEALED_VALUE_BYTES || value.contains('\0') {
                return Err(BoundError::TooLong {
                    field: "sealed_environment".to_owned(),
                    maximum: MAX_SEALED_VALUE_BYTES,
                }
                .into());
            }
        }
        Ok(spec)
    }
}

fn bounded_text(field: &str, value: &str, maximum: usize) -> Result<(), BoundError> {
    if value.is_empty() {
        return Err(BoundError::Empty {
            field: field.to_owned(),
        });
    }
    if value.len() > maximum || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(BoundError::TooLong {
            field: field.to_owned(),
            maximum,
        });
    }
    Ok(())
}
