//! Human-readable rendering of rejections and invalid-value reasons.

use std::fmt;

use super::{InvalidReason, Rejection};

impl fmt::Display for InvalidReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zero => formatter.write_str("must not be zero"),
            Self::ExceedsMaximum { maximum } => write!(formatter, "exceeds maximum {maximum}"),
            Self::Empty => formatter.write_str("must not be empty"),
            Self::NotAbsolutePath => formatter.write_str("must be an absolute guest path"),
            Self::NotNormalizedPath => formatter.write_str("must be a normalized guest path"),
            Self::InvalidUser => formatter.write_str("must be a portable user name"),
            Self::InvalidMode => formatter.write_str("must be an owner-only file mode"),
            Self::InvalidPort => formatter.write_str("must be a port between 1 and 65535"),
            Self::InvalidTimeout => formatter.write_str("must be a bounded nonzero timeout"),
            Self::TimeoutOrdering => {
                formatter.write_str("idle timeout must not exceed the maximum lifetime")
            }
            Self::InvalidDomain => formatter.write_str("must be a lowercase domain name"),
            Self::InvalidCidr => formatter.write_str("must be an IPv4 or IPv6 CIDR"),
            Self::ContradictoryEgress => {
                formatter.write_str("unrestricted egress cannot carry an allowlist")
            }
            Self::EmptyAllowlist => formatter.write_str("allowlist egress needs a destination"),
            Self::Duplicate => formatter.write_str("is declared more than once"),
            Self::DestinationNotAllowed => {
                formatter.write_str("names a destination outside the network envelope")
            }
            Self::ForbiddenCharacter => formatter.write_str("contains a forbidden character"),
        }
    }
}

pub(super) fn rejection(rejection: &Rejection, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match rejection {
        Rejection::DuplicateExclusiveOwnership { .. }
        | Rejection::ConflictingDefaultCommands { .. }
        | Rejection::ConflictingSealedEnvironment { .. }
        | Rejection::MissingDefaultCommand { .. }
        | Rejection::ModuleCycle { .. }
        | Rejection::UnpinnedInput { .. }
        | Rejection::UnknownModule { .. }
        | Rejection::DuplicateModule { .. } => structural(rejection, formatter),
        _ => policy(rejection, formatter),
    }
}

fn structural(rejection: &Rejection, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    let field = rejection.field();
    match rejection {
        Rejection::DuplicateExclusiveOwnership {
            first,
            second,
            owned,
            ..
        } => write!(
            formatter,
            "{second} {field}: `{owned}` is already owned by {first}"
        ),
        Rejection::ConflictingDefaultCommands { first, second, .. } => write!(
            formatter,
            "{second} {field}: conflicts with the default command of {first}"
        ),
        Rejection::ConflictingSealedEnvironment {
            module,
            conflicting_module: Some(other),
            name,
            ..
        } => write!(
            formatter,
            "{module} {field}: `{name}` is sealed with a different value by {other}"
        ),
        Rejection::ConflictingSealedEnvironment { module, name, .. } => {
            write!(formatter, "{field}: `{name}` is sealed by {module}")
        }
        Rejection::MissingDefaultCommand { .. } => {
            write!(formatter, "{field}: no module supplies a default command")
        }
        Rejection::ModuleCycle { module, cycle, .. } => {
            write!(formatter, "{module} {field}: module cycle")?;
            for member in cycle {
                write!(formatter, " -> {member}")?;
            }
            Ok(())
        }
        Rejection::UnpinnedInput {
            module, reference, ..
        } => with_module(formatter, module.as_ref(), field, reference, "is unpinned"),
        Rejection::UnknownModule {
            module, reference, ..
        } => with_module(formatter, module.as_ref(), field, reference, "is unknown"),
        Rejection::DuplicateModule { reference, .. } => {
            write!(formatter, "{field}: `{reference}` is listed more than once")
        }
        _ => policy(rejection, formatter),
    }
}

fn policy(rejection: &Rejection, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    let field = rejection.field();
    match rejection {
        Rejection::UnresolvableImage {
            reference,
            platform,
            ..
        } => write!(
            formatter,
            "{field}: image `{reference}` cannot be resolved for platform {platform}"
        ),
        Rejection::UnsupportedPlatform {
            module: Some(module),
            platform,
            ..
        } => write!(
            formatter,
            "{module} {field}: platform {platform} unsupported"
        ),
        Rejection::UnsupportedPlatform { platform, .. } => write!(
            formatter,
            "{field}: platform {platform} unsupported by the Backend"
        ),
        Rejection::MissingRequiredEnvironment { module, name, .. } => write!(
            formatter,
            "{module} {field}: environment `{name}` is not provided"
        ),
        Rejection::SecretLiteral { name, .. } => {
            write!(formatter, "{field}: `{name}` carries a secret literal")
        }
        Rejection::SecretWithoutScope { name, .. } => {
            write!(formatter, "{field}: secret `{name}` lacks a delivery scope")
        }
        Rejection::NetworkExceedsCeiling {
            requested, ceiling, ..
        } => write!(
            formatter,
            "{field}: `{requested}` exceeds the policy ceiling `{ceiling}`"
        ),
        Rejection::ExecutableAbsent { program, .. } => write!(
            formatter,
            "{field}: executable `{program}` is absent from the resolved filesystem"
        ),
        Rejection::InvalidValue {
            module: Some(module),
            reason,
            ..
        } => write!(formatter, "{module} {field}: {reason}"),
        Rejection::InvalidValue { reason, .. } => write!(formatter, "{field}: {reason}"),
        Rejection::UnsupportedLifecycleAction { action, .. } => {
            write!(
                formatter,
                "{field}: `{action}` is unsupported by the Backend"
            )
        }
        _ => structural(rejection, formatter),
    }
}

fn with_module(
    formatter: &mut fmt::Formatter<'_>,
    module: Option<&crate::module::ModuleIdentity>,
    field: &str,
    reference: &str,
    verdict: &str,
) -> fmt::Result {
    match module {
        Some(module) => write!(formatter, "{module} {field}: `{reference}` {verdict}"),
        None => write!(formatter, "{field}: `{reference}` {verdict}"),
    }
}
