use core::fmt;

/// A redacted authenticated-session failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Error {
    /// A required identity or nonce was all zeroes.
    InvalidBinding,
    /// A key had an invalid length or an all-zero value.
    InvalidKeyMaterial,
    /// An X25519 public key produced a non-contributory all-zero shared output.
    NonContributoryPublicKey,
    /// A PSK was provisioned for a different concrete Instance.
    PskInstanceMismatch,
    /// The selected cryptographic suite could not be initialized.
    CryptoSetup,
    /// A framed handshake message was malformed or exceeded its bound.
    HandshakeRejected,
    /// The peer did not complete the authenticated handshake.
    AuthenticationFailed,
    /// A caller payload exceeded the encrypted-record bound.
    RecordTooLarge,
    /// The peer supplied a malformed, unauthenticated, or out-of-order record.
    PeerRecordRejected,
    /// The session rejected all further peer input after a prior peer error.
    SessionPoisoned,
    /// A directional sequence or cipher nonce could not advance safely.
    SessionExhausted,
    /// The operating system did not provide fresh launch randomness.
    RandomnessUnavailable,
    /// A launch page was not the one exact supported encoding.
    LaunchPageRejected,
    /// A launch network identity violated the fixed address, prefix, or transport bounds.
    InvalidLaunchNetwork,
    /// An operation identity used the reserved all-zero value.
    InvalidOperation,
    /// A local direct command violated the bounded wire contract.
    InvalidCommand,
    /// A local application message exceeded one authenticated record.
    ApplicationMessageTooLarge,
    /// A peer application message was not the one exact supported encoding.
    ApplicationMessageRejected,
    /// A local output chunk violated the bounded streaming contract.
    InvalidOutputChunk,
    /// A local terminal status violated the Linux process-result contract.
    InvalidTerminalStatus,
    /// A local terminal report violated the authenticated output-count contract.
    InvalidTerminalReport,
    /// An activation scope named a zero identity, generation, or intent digest.
    InvalidActivationScope,
    /// An activation receipt did not authenticate against the challenge and scope.
    ActivationReceiptRejected,
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBinding => "invalid session binding",
            Self::InvalidKeyMaterial => "invalid key material",
            Self::NonContributoryPublicKey => "non-contributory X25519 public key",
            Self::PskInstanceMismatch => "PSK Instance binding mismatch",
            Self::CryptoSetup => "cryptographic setup failed",
            Self::HandshakeRejected => "handshake message rejected",
            Self::AuthenticationFailed => "peer authentication failed",
            Self::RecordTooLarge => "record payload exceeds the bound",
            Self::PeerRecordRejected => "peer record rejected",
            Self::SessionPoisoned => "session is poisoned",
            Self::SessionExhausted => "session sequence exhausted",
            Self::RandomnessUnavailable => "operating-system randomness unavailable",
            Self::LaunchPageRejected => "launch page rejected",
            Self::InvalidLaunchNetwork => "invalid launch network identity",
            Self::InvalidOperation => "invalid operation identity",
            Self::InvalidCommand => "invalid guest command",
            Self::ApplicationMessageTooLarge => "application message exceeds one record",
            Self::ApplicationMessageRejected => "application message rejected",
            Self::InvalidOutputChunk => "invalid output chunk",
            Self::InvalidTerminalStatus => "invalid terminal status",
            Self::InvalidTerminalReport => "invalid terminal report",
            Self::InvalidActivationScope => "invalid network activation scope",
            Self::ActivationReceiptRejected => "network activation receipt rejected",
        })
    }
}

impl std::error::Error for Error {}
