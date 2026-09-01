//! Automatic local backend selection without weakening explicit requests.

use crate::{LocalFailure, LocalFailureKind};

use super::BackendSelection;

pub(super) fn resolve_selection(
    selection: BackendSelection,
) -> Result<BackendSelection, LocalFailure> {
    if selection != BackendSelection::Auto {
        return Ok(selection);
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        if super::docker::is_available() {
            return Ok(BackendSelection::Docker);
        }
        return Ok(BackendSelection::Macos);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok(BackendSelection::Kvm);
    }
    #[allow(unreachable_code)]
    Err(LocalFailure::new(LocalFailureKind::UnsupportedTarget))
}
