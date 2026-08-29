//! Shape rules applied to decoded lock records, mirroring the validator.
//!
//! Every rule here re-checks a value the validator checked before encoding, so a byte
//! substitution that keeps a lock well-formed on the wire but invalid as a Template is
//! rejected at decode time and the revision view never sees it.

use super::{LockedCommand, LockedEnvironment, LockedSecret};
use crate::{
    error::LockError,
    module::{EnvironmentName, GuestPath},
    resolve::ResolvedImage,
    schema::{Lifecycle, Resources, SecretDelivery},
    validate::{BackendCapabilities, NetworkEnvelope, secret, syntax},
};

const fn invalid(field: &'static str) -> LockError {
    LockError::InvalidField { field }
}

/// The resolved platform must be one the bound Backend supports.
pub(super) fn image(image: &ResolvedImage, backend: &BackendCapabilities) -> Result<(), LockError> {
    if backend.supports_platform(image.platform()) {
        Ok(())
    } else {
        Err(invalid("workload.platform"))
    }
}

/// A non-empty control-free program, NUL-free arguments, an absolute normalized working
/// directory, and a portable user.
pub(super) fn command(command: &LockedCommand) -> Result<(), LockError> {
    let well_formed = !command.program.is_empty()
        && !command.program.bytes().any(|byte| byte.is_ascii_control())
        && !command.args.iter().any(|argument| argument.contains('\0'))
        && GuestPath::parse(&command.working_directory).is_ok()
        && syntax::user(&command.user).is_ok();
    if well_formed {
        Ok(())
    } else {
        Err(invalid("command"))
    }
}

/// Every dimension is nonzero and within the bound Backend limits.
pub(super) fn resources(
    resources: &Resources,
    backend: &BackendCapabilities,
) -> Result<(), LockError> {
    let limits = backend.limits();
    let within = |value: u64, maximum: u64| value != 0 && value <= maximum;
    let max_vcpus = u64::from(limits.max_vcpus.min(u32::from(u16::MAX)));
    if within(u64::from(resources.vcpus), max_vcpus)
        && within(resources.memory_mib, limits.max_memory_mib)
        && within(
            resources.writable_storage_mib,
            limits.max_writable_storage_mib,
        )
    {
        Ok(())
    } else {
        Err(invalid("resources"))
    }
}

/// Bounded nonzero timeouts, idle not above maximum, and an action the Backend supports.
pub(super) fn lifecycle(
    lifecycle: &Lifecycle,
    backend: &BackendCapabilities,
) -> Result<(), LockError> {
    let well_formed = syntax::timeout(lifecycle.idle_timeout_seconds).is_ok()
        && syntax::timeout(lifecycle.maximum_lifetime_seconds).is_ok()
        && lifecycle.idle_timeout_seconds <= lifecycle.maximum_lifetime_seconds
        && backend.supports_idle_action(lifecycle.on_idle);
    if well_formed {
        Ok(())
    } else {
        Err(invalid("lifecycle"))
    }
}

/// Portable names in strictly increasing order, NUL-free values, and a value behind every
/// seal.
pub(super) fn environment(entries: &[LockedEnvironment]) -> Result<(), LockError> {
    for entry in entries {
        let well_formed = EnvironmentName::parse(&entry.name).is_ok()
            && !entry
                .value
                .as_deref()
                .is_some_and(|value| value.contains('\0'))
            && !(entry.sealed_by.is_some() && entry.value.is_none());
        if !well_formed {
            return Err(invalid("environment"));
        }
    }
    if sorted_unique(entries.iter().map(|entry| entry.name.as_str())) {
        Ok(())
    } else {
        Err(invalid("environment"))
    }
}

/// Portable names in strictly increasing order, a `secret://` source, a scope of the shape
/// its delivery needs inside the envelope, and a mode exactly for file delivery.
pub(super) fn secrets(
    secrets: &[LockedSecret],
    envelope: &NetworkEnvelope,
) -> Result<(), LockError> {
    for secret in secrets {
        let scope_valid = match secret.delivery {
            SecretDelivery::Environment => EnvironmentName::parse(&secret.scope).is_ok(),
            SecretDelivery::File => GuestPath::parse(&secret.scope).is_ok(),
            SecretDelivery::EgressProxy => {
                syntax::domain(&secret.scope).is_ok() && envelope.allows_domain(&secret.scope)
            }
        };
        let mode_valid = match (secret.delivery, secret.mode) {
            (SecretDelivery::File, Some(mode)) => syntax::secret_mode(mode).is_ok(),
            (SecretDelivery::File, None) | (_, Some(_)) => false,
            (_, None) => true,
        };
        let well_formed = EnvironmentName::parse(&secret.name).is_ok()
            && secret::secret_source_shape(&secret.source)
            && scope_valid
            && mode_valid;
        if !well_formed {
            return Err(invalid("secrets"));
        }
    }
    if sorted_unique(secrets.iter().map(|secret| secret.name.as_str())) {
        Ok(())
    } else {
        Err(invalid("secrets"))
    }
}

/// Whether names are strictly increasing, the one order the encoder emits.
fn sorted_unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let names: Vec<&str> = names.collect();
    names.windows(2).all(|pair| pair[0] < pair[1])
}
