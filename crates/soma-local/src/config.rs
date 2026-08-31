use std::{env, path::PathBuf};

use crate::{BackendSelection, LocalFailure, LocalFailureKind};

#[derive(Clone, PartialEq, Eq)]
pub struct LocalRuntimeConfig {
    pub(crate) backend: BackendSelection,
    pub(crate) runtime: Option<PathBuf>,
    pub(crate) state_root: PathBuf,
    pub(crate) hosted_machines: bool,
}

impl std::fmt::Debug for LocalRuntimeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalRuntimeConfig")
            .field("backend", &self.backend)
            .field("runtime", &self.runtime.as_ref().map(|_| "[REDACTED]"))
            .field("state_root", &"[REDACTED]")
            .field("hosted_machines", &self.hosted_machines)
            .finish()
    }
}

impl LocalRuntimeConfig {
    #[must_use]
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            backend: BackendSelection::Auto,
            runtime: None,
            state_root: state_root.into(),
            hosted_machines: false,
        }
    }

    /// Requires machines launched through this runtime to outlive the process that launched them.
    ///
    /// Only the managed Machine lifecycle asks for this. A one-shot run holds its machine in its
    /// own process for the whole operation and releases it before returning, so hosting one would
    /// add a process to a path that never needs a second one.
    #[must_use]
    pub const fn with_hosted_machines(mut self, hosted: bool) -> Self {
        self.hosted_machines = hosted;
        self
    }

    #[must_use]
    pub const fn with_backend(mut self, backend: BackendSelection) -> Self {
        self.backend = backend;
        self
    }

    #[must_use]
    pub fn with_runtime(mut self, runtime: Option<PathBuf>) -> Self {
        self.runtime = runtime;
        self
    }

    /// Resolves the one shared per-user state root used by local CLI and MCP callers.
    ///
    /// Explicit state and runtime paths always win. No SOMA-specific environment fallback is
    /// consulted.
    ///
    /// # Errors
    ///
    /// Returns a typed configuration failure when no absolute per-user state root is available.
    pub fn discover(
        backend: BackendSelection,
        runtime: Option<PathBuf>,
        explicit_state_root: Option<PathBuf>,
    ) -> Result<Self, LocalFailure> {
        let state_root = match explicit_state_root {
            Some(root) => root,
            None => default_state_root()?,
        };
        if state_root.as_os_str().is_empty() || !state_root.is_absolute() {
            return Err(LocalFailure::new(LocalFailureKind::InvalidConfiguration));
        }
        Ok(Self::new(state_root)
            .with_backend(backend)
            .with_runtime(runtime))
    }

    #[must_use]
    pub fn state_root(&self) -> &std::path::Path {
        &self.state_root
    }
}

fn default_state_root() -> Result<PathBuf, LocalFailure> {
    #[cfg(target_os = "macos")]
    {
        return home_directory().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("SOMA")
                .join("state")
                .join("v1")
        });
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(root) = env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
            return absolute(root).map(|root| root.join("SOMA").join("v1"));
        }
        return home_directory()
            .map(|home| home.join(".local").join("state").join("SOMA").join("v1"));
    }
    #[cfg(target_os = "windows")]
    {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(invalid_configuration)
            .and_then(absolute)
            .map(|root| root.join("SOMA").join("state").join("v1"));
    }
    #[allow(unreachable_code)]
    Err(invalid_configuration())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn home_directory() -> Result<PathBuf, LocalFailure> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(invalid_configuration)
        .and_then(absolute)
}

fn absolute(path: PathBuf) -> Result<PathBuf, LocalFailure> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(invalid_configuration())
    }
}

const fn invalid_configuration() -> LocalFailure {
    LocalFailure::new(LocalFailureKind::InvalidConfiguration)
}
