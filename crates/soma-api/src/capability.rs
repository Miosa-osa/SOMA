use crate::envelope::ApiError;

/// A capability the provider contract asks for that the SOMA engine cannot perform today.
///
/// These are refusals, not failures. Each variant names one concrete engine capability that is
/// absent, so a client integrating against this service learns exactly what is missing instead of
/// receiving an empty list or an invented success that it would then trust.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingCapability {
    /// `StateStore` addresses records by exact Instance ID and exposes no enumeration, so the set
    /// of live sandboxes cannot be read back out of durable state by any process.
    SandboxEnumeration,
    /// `soma-guest` does implement guest filesystem operations at the protocol level, but the
    /// portable facade's `Backend` trait carries only resolve, launch, execute, inspect, and
    /// cleanup, so no engine call reaches them and this service has nothing to drive.
    GuestFilesystemTransfer,
}

impl MissingCapability {
    #[must_use]
    pub const fn error(self) -> ApiError {
        ApiError::new(501, "capability_unavailable", self.message(), false)
    }

    /// Names the missing engine capability in the failure message.
    ///
    /// The message names the engine-side capability rather than the HTTP route, because the route
    /// is what the caller already knows and the capability is what has to be built.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::SandboxEnumeration => {
                "the SOMA durable state store cannot enumerate sandboxes; \
                 it resolves records only by exact instance id"
            }
            Self::GuestFilesystemTransfer => {
                "the SOMA portable facade exposes no guest filesystem transfer; \
                 the guest protocol implements one, but no backend or engine method reaches it"
            }
        }
    }
}
