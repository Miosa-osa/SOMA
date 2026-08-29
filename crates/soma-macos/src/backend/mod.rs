mod command;
mod image;
mod lifecycle;
mod network;
mod one_shot;
mod ownership;
mod probe;

use std::path::PathBuf;

#[cfg(test)]
use std::sync::Arc;

use crate::BackendError;
#[cfg(test)]
use crate::process::ProcessRunner;

use self::command::CommandExecutor;

/// A development-only local OCI sandbox adapter for macOS.
///
/// Every command is invoked directly without a shell.
/// Production Linux KVM certification remains outside this adapter.
pub struct MacOsBackend {
    commands: CommandExecutor,
    host_supported: bool,
}

impl MacOsBackend {
    /// Uses `container` from `PATH` on a supported macOS host.
    #[must_use]
    pub fn new() -> Self {
        Self::with_executable("container")
    }

    /// Uses an explicitly selected Apple `container` executable.
    #[must_use]
    pub fn with_executable(executable: impl Into<PathBuf>) -> Self {
        Self {
            commands: CommandExecutor::system(executable.into()),
            host_supported: cfg!(all(target_os = "macos", target_arch = "aarch64")),
        }
    }

    pub(super) const fn ensure_host(&self) -> Result<(), BackendError> {
        if self.host_supported {
            Ok(())
        } else {
            Err(BackendError::UnsupportedHost)
        }
    }

    #[cfg(test)]
    pub(crate) fn with_runner(
        executable: impl Into<PathBuf>,
        runner: Arc<dyn ProcessRunner>,
    ) -> Self {
        Self {
            commands: CommandExecutor::new(executable.into(), runner),
            host_supported: true,
        }
    }
}

impl Default for MacOsBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for MacOsBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MacOsBackend")
            .field("backend_class", &"development_only")
            .field("host_supported", &self.host_supported)
            .finish_non_exhaustive()
    }
}
