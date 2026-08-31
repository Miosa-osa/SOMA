//! Authenticated guest-control protocol and semantic session owner.
//!
//! This crate owns the cryptographic session, byte transport, launch-page bytes, bounded
//! application codec, repair gate, and one-operation exchange state.
//! It does not map a confidential non-snapshot page, perform guest repair, execute a process,
//! or attest a Generation.
//!
//! Raw PSKs and handshake factories are intentionally not public API.
//!
//! ```compile_fail
//! use soma_guest::InstancePsk;
//! ```
//!
//! ```compile_fail
//! use soma_guest::InitiatorHandshake;
//! ```
//!
//! ```compile_fail
//! use soma_guest::ResponderHandshake;
//! ```
//!
//! ```compile_fail
//! use soma_guest::InitiatorAwaitingResponse;
//! ```
//!
//! ```compile_fail
//! use soma_guest::ResponderPendingResponse;
//! ```
//!
//! ```compile_fail
//! use soma_guest::AuthenticatedSession;
//! ```

#![forbid(unsafe_code)]

mod activation;
mod application;
mod binding;
mod control;
mod error;
mod handshake;
mod launch_page;
mod record;
mod resolver;
mod secret;

pub use activation::{ActivationChallenge, ActivationReceipt, ActivationScope};
pub use application::{
    CommandContext, DirectoryEntry, EntryKind, EnvironmentPair, FileFailure, FileOutcome,
    FileRequest, GuestCommand, GuestMessage, HostMessage, MAX_CHUNK_BYTES, MAX_ENTRIES,
    MAX_PATH_BYTES, MAX_PTY_CHUNK_BYTES, MAX_PTY_COLUMNS, MAX_PTY_ROWS, MAX_PTY_WAIT_MILLIS,
    OperationId, OutputChunk, PtyFailure, PtyOutcome, PtyRequest, PtySize, TerminalReport,
    TerminalStatus,
};
pub use binding::SessionBinding;
pub use control::{
    CONTROL_VSOCK_PORT, ControlError, ControlFailureClass, ControlIo, ControlStage, ExecuteOutcome,
    GuestControl, GuestRequest, HostControl, HostControlIo, RepairedHostControl, WholeFileRead,
    WholeFileWrite,
};
pub use error::Error;
pub(crate) use handshake::{
    InitiatorAwaitingResponse, InitiatorHandshake, ResponderHandshake, ResponderPendingResponse,
};
pub use launch_page::{
    DeliveredHostLaunchMaterial, GuestLaunchMaterial, GuestSessionMaterial, HostLaunchMaterial,
    LAUNCH_PAGE_GUEST_ADDRESS, LAUNCH_PAGE_SCHEMA_VERSION, LAUNCH_PAGE_SIZE, LaunchNetwork,
};
pub(crate) use record::AuthenticatedSession;
pub use record::MAX_RECORD_PAYLOAD;
pub(crate) use secret::InstancePsk;
pub use secret::{ResponderKeypair, ResponderPrivateKey, ResponderPublicKey};

const NOISE_PATTERN: &str = "Noise_NKpsk0_25519_ChaChaPoly_BLAKE2s";
