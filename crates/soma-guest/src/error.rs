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
        })
    }
}

impl std::error::Error for Error {}
