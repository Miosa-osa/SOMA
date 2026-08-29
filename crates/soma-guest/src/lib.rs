//! Test-oriented authenticated guest-control building blocks.
//!
//! This crate establishes a cryptographic protocol seam.
//! It does not establish production key custody, snapshot-safe secret injection, guest
//! attestation, or SOMA readiness.

#![forbid(unsafe_code)]

mod binding;
mod error;
mod handshake;
mod record;
mod resolver;
mod secret;

pub use binding::SessionBinding;
pub use error::Error;
pub use handshake::{
    InitiatorAwaitingResponse, InitiatorHandshake, ResponderHandshake, ResponderPendingResponse,
};
pub use record::{AuthenticatedSession, MAX_RECORD_PAYLOAD};
pub use secret::{InstancePsk, ResponderKeypair, ResponderPrivateKey, ResponderPublicKey};

const NOISE_PATTERN: &str = "Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s";
