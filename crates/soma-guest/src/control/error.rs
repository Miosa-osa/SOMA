use core::fmt;

/// Authenticated control stage that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlStage {
    /// The fixed two-message Noise handshake.
    Handshake,
    /// Authenticated guest repair and its host commit gate.
    Repair,
    /// One direct command exchange.
    Execute,
    /// One bounded filesystem request and its outcome.
    File,
    /// One interactive terminal request and its outcome.
    Pty,
    /// Graceful guest-agent shutdown.
    Shutdown,
}

/// Redacted class of authenticated control failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlFailureClass {
    /// The owned byte transport or repair-commit adapter failed.
    Io,
    /// Peer authentication or encrypted-record verification failed.
    Authentication,
    /// Authenticated bytes violated the fixed wire contract.
    Protocol,
    /// A local or peer operation violated the lifecycle state.
    Lifecycle,
    /// Authenticated output violated allowance or count invariants.
    Accounting,
}

/// A redacted terminal failure from an authenticated control owner.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ControlError {
    stage: ControlStage,
    class: ControlFailureClass,
}

impl ControlError {
    pub(crate) const fn new(stage: ControlStage, class: ControlFailureClass) -> Self {
        Self { stage, class }
    }

    /// Returns the stage that failed without exposing peer-controlled bytes.
    #[must_use]
    pub const fn stage(self) -> ControlStage {
        self.stage
    }

    /// Returns the redacted failure class.
    #[must_use]
    pub const fn class(self) -> ControlFailureClass {
        self.class
    }
}

impl fmt::Debug for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlError")
            .field("stage", &self.stage)
            .field("class", &self.class)
            .finish()
    }
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "control {:?} failed: {:?}",
            self.stage, self.class
        )
    }
}

impl std::error::Error for ControlError {}
