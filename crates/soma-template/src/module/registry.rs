//! A bounded in-memory module registry.
//!
//! The registry is the only place composition looks up module data.
//! A remote or content-addressed module store is a later slice and will implement the same
//! lookup behind this type rather than a second resolution path.

use std::collections::BTreeMap;

use super::{ModuleError, ModuleIdentity, ModuleSpec, builtin};

pub const MAX_REGISTRY_MODULES: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct ModuleRegistry {
    modules: BTreeMap<ModuleIdentity, ModuleSpec>,
}

impl ModuleRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The registry holding every built-in example module.
    #[must_use]
    pub fn builtin() -> Self {
        builtin::registry()
    }

    /// Adds one module version.
    ///
    /// # Errors
    ///
    /// Returns [`ModuleError::DuplicateModule`] when the identity is already registered and
    /// [`ModuleError::RegistryFull`] at [`MAX_REGISTRY_MODULES`].
    pub fn register(&mut self, spec: ModuleSpec) -> Result<(), ModuleError> {
        if self.modules.contains_key(spec.identity()) {
            return Err(ModuleError::DuplicateModule(spec.identity().clone()));
        }
        if self.modules.len() >= MAX_REGISTRY_MODULES {
            return Err(ModuleError::RegistryFull {
                maximum: MAX_REGISTRY_MODULES,
            });
        }
        self.modules.insert(spec.identity().clone(), spec);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, identity: &ModuleIdentity) -> Option<&ModuleSpec> {
        self.modules.get(identity)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.modules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn identities(&self) -> impl Iterator<Item = &ModuleIdentity> {
        self.modules.keys()
    }
}
