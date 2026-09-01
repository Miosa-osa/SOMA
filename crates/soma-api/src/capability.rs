use crate::envelope::ApiError;

/// A capability the provider contract asks for that the SOMA engine cannot perform today.
///
/// These are refusals, not failures. Each variant names one concrete engine capability that is
/// absent, so a client integrating against this service learns exactly what is missing instead of
/// receiving an empty list or an invented success that it would then trust.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingCapability {
    /// The path from this service to the guest's filesystem exists and is served by the KVM
    /// backend. What remains missing is a machine to address: a backend whose machines do not
    /// outlive the process that launched them has nothing for a later filesystem call to reach,
    /// and says so here rather than answering about a sandbox that is already gone.
    GuestFilesystemTransfer,
    /// The path from this service to the guest's terminal exists and is served by the KVM
    /// backend. What remains missing is a session to address: a backend whose machines do not
    /// outlive the process that launched them has nowhere for a terminal to stay open between two
    /// requests, so the second one would reach a session that no longer exists.
    GuestTerminalSession,
    /// The backend hosts a Machine inside the process that launched it, and this service opens
    /// one runtime per connection, so a created sandbox identity would name a Machine that is
    /// gone before the caller can address it again.
    DurableMachineHosting,
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
            Self::GuestFilesystemTransfer => {
                "this backend holds no machine that outlives the process which launched it, \
                 so a guest filesystem operation has no sandbox left to address"
            }
            Self::GuestTerminalSession => {
                "this backend holds no machine that outlives the process which launched it, \
                 so a terminal session has nowhere to stay open between two requests"
            }
            Self::DurableMachineHosting => {
                "this backend hosts a machine inside the process that launched it, and this \
                 service opens one runtime per connection, so the sandbox identity a create \
                 would return could not be addressed by any later request"
            }
        }
    }
}
