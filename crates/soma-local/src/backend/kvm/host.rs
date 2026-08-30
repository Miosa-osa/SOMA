//! The host inputs a KVM Backend needs before it can compile or boot anything.
//!
//! A Generation is built from artifacts this crate does not ship: a pinned kernel, the static
//! guest agent, and the filesystem tools that format the root and the overlay template. The
//! portable lifecycle carries none of them, because they are properties of the host rather than
//! of the request, so they are resolved once when the Backend opens and the Backend refuses to
//! open without them. Resolving them later would turn a host misconfiguration into a failed
//! sandbox instead of an unavailable Backend.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Absolute path to the pinned PVH kernel image.
pub(super) const KERNEL: &str = "SOMA_X86_64_VMLINUX";
/// Absolute path to the exact kernel configuration text the manifest binds.
pub(super) const KERNEL_CONFIG: &str = "SOMA_X86_64_KERNEL_CONFIG";
/// Absolute path to the static `soma-guest-agent` built for `x86_64-unknown-linux-musl`.
pub(super) const GUEST_AGENT: &str = "SOMA_GUEST_AGENT";
/// Directory holding the pinned `erofs-utils` binaries.
pub(super) const EROFS_TOOLS: &str = "SOMA_EROFS_TOOLS";
/// Directory holding the `e2fsprogs` binaries that format the overlay template.
pub(super) const E2FSPROGS: &str = "SOMA_E2FSPROGS";

/// Why a host cannot serve as a KVM Backend.
///
/// The variants name the condition rather than the path, so a caller can report a precise
/// reason without a host layout appearing in a message that may be logged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum HostInputError {
    /// `/dev/kvm` is absent or cannot be opened for read and write.
    NoKvm,
    /// A required variable is unset.
    Unset(&'static str),
    /// A variable is set but does not name an existing file or directory.
    NotFound(&'static str),
    /// A variable is set to a relative path.
    ///
    /// A Backend resolves artifacts long after the working directory that named them, so a
    /// relative path is refused rather than resolved against whatever directory is current.
    NotAbsolute(&'static str),
}

/// Every host artifact the KVM Backend binds for its whole life.
///
/// The fields are read by Generation preparation, which is a separate step from the request
/// path: a prepared host compiles Generations ahead of demand and the Backend resolves against
/// what is already prepared.
#[derive(Clone, Debug)]
#[allow(
    dead_code,
    reason = "consumed by Generation preparation, which is not yet wired"
)]
pub(super) struct HostInputs {
    pub(super) kernel: PathBuf,
    pub(super) kernel_config: PathBuf,
    pub(super) guest_agent: PathBuf,
    pub(super) erofs_tools: PathBuf,
    pub(super) e2fsprogs: PathBuf,
}

/// The kernel configuration that sits beside a kernel image, when the operator names no other.
///
/// The compiler binds the exact configuration text, so a kernel whose configuration cannot be
/// found is refused rather than compiled against a guess.
fn default_kernel_config(kernel: &Path) -> PathBuf {
    kernel.with_file_name("final.config")
}

/// Classifies one already-read value, so the rules are testable without touching the process.
///
/// Mutating the environment to test these rules would make the outcome depend on test ordering
/// inside one process, which is exactly the kind of hidden coupling this Backend must not have.
fn classify(
    variable: &'static str,
    value: Option<&OsStr>,
    directory: bool,
) -> Result<PathBuf, HostInputError> {
    let path = PathBuf::from(value.ok_or(HostInputError::Unset(variable))?);
    if !path.is_absolute() {
        return Err(HostInputError::NotAbsolute(variable));
    }
    let present = if directory {
        path.is_dir()
    } else {
        path.is_file()
    };
    if present {
        Ok(path)
    } else {
        Err(HostInputError::NotFound(variable))
    }
}

fn required(variable: &'static str, directory: bool) -> Result<PathBuf, HostInputError> {
    classify(variable, std::env::var_os(variable).as_deref(), directory)
}

impl HostInputs {
    /// Resolves every host input, or names the first condition that fails.
    pub(super) fn resolve() -> Result<Self, HostInputError> {
        if !Path::new("/dev/kvm").exists() {
            return Err(HostInputError::NoKvm);
        }
        let kernel = required(KERNEL, false)?;
        let named = std::env::var_os(KERNEL_CONFIG);
        let kernel_config = if named.is_some() {
            classify(KERNEL_CONFIG, named.as_deref(), false)?
        } else {
            let beside = default_kernel_config(&kernel);
            if !beside.is_file() {
                return Err(HostInputError::NotFound(KERNEL_CONFIG));
            }
            beside
        };
        Ok(Self {
            kernel,
            kernel_config,
            guest_agent: required(GUEST_AGENT, false)?,
            erofs_tools: required(EROFS_TOOLS, true)?,
            e2fsprogs: required(E2FSPROGS, true)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_refused_rather_than_resolved() {
        assert_eq!(
            classify(KERNEL, Some(OsStr::new("relative/vmlinux")), false)
                .expect_err("a relative path must be refused"),
            HostInputError::NotAbsolute(KERNEL)
        );
    }

    #[test]
    fn an_unset_variable_names_itself() {
        assert_eq!(
            classify(E2FSPROGS, None, true).expect_err("an unset variable must be refused"),
            HostInputError::Unset(E2FSPROGS)
        );
    }

    #[test]
    fn an_absolute_path_that_does_not_exist_is_refused() {
        assert_eq!(
            classify(
                GUEST_AGENT,
                Some(OsStr::new("/nonexistent/soma-guest-agent")),
                false
            )
            .expect_err("a missing artifact must be refused"),
            HostInputError::NotFound(GUEST_AGENT)
        );
    }

    #[test]
    fn a_file_is_not_accepted_where_a_directory_is_required() {
        assert_eq!(
            classify(EROFS_TOOLS, Some(OsStr::new("/etc/hostname")), true)
                .expect_err("a file must not satisfy a directory input"),
            HostInputError::NotFound(EROFS_TOOLS)
        );
    }

    #[test]
    fn the_kernel_configuration_defaults_beside_the_kernel_image() {
        assert_eq!(
            default_kernel_config(Path::new("/opt/soma/vmlinux-6.12.107-soma-v1")),
            Path::new("/opt/soma/final.config")
        );
    }
}
