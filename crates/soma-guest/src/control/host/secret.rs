//! Placing one secret into a running Instance over the session it already authenticated.
//!
//! Nothing new is invented for a secret. The bounded filesystem request the session already
//! carries is exactly the mechanism a secret needs, and it is the only one that reaches the guest
//! after the handshake: the launch page is public and consumed before the guest agent starts,
//! and the snapshot is shared by every Instance of a Generation. A second, secret-only path
//! would be a second thing to authenticate, bound, and audit, for no capability this one lacks.
//!
//! The order of the steps is the security property. The file is created exclusively at a private
//! mode before it holds anything, so the value never exists at a mode the guest's ambient mask
//! chose, and no file that was already at the destination is written into. The value follows.
//! The requested mode is applied last, because a mode with no write permission is exactly what a
//! delivered credential should end at, and applying it first would leave the write to depend on
//! the guest agent's privilege.
//!
//! Every step is fail-closed. A refusal is reported as a refusal, not as a delivered secret, so
//! a caller cannot mistake a sandbox without its credential for one that has it.

use crate::{FileFailure, FileOutcome, FileRequest, MAX_SECRET_BYTES, SecretFile};

use super::super::error::ControlError;
use super::{HostControlIo, RepairedHostControl, WholeFileWrite};

/// The mode a destination is created at, before its value and its final mode arrive.
///
/// Owner read and write: the write needs the write bit, and nothing beyond the owner may read a
/// file that is about to hold a credential, not even for the moment the transfer takes.
const PRIVATE_CREATE_MODE: u32 = 0o600;

/// Which step of a delivery the guest refused.
///
/// The stage is evidence a caller can act on and carries nothing about the value or the path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretStage {
    /// Making the directory the destination lives in.
    Directory,
    /// Creating the destination file itself.
    Create,
    /// Writing the value.
    Write,
    /// Applying the destination's final mode.
    Mode,
}

/// How one secret delivery ended.
///
/// A refusal is not a [`ControlError`]: the guest answered, and the transport is still usable.
/// It is still a failed delivery, and a launch that sees one must not continue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum SecretPlacement {
    /// The destination exists, holds the value, and is at the mode that was asked for.
    Placed,
    /// The guest declined one step, and no later step was attempted.
    Refused {
        /// The step the guest declined.
        stage: SecretStage,
        /// What the guest said about it.
        failure: FileFailure,
    },
}

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Places one secret into the running Instance and returns the reusable owner.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once. A guest that
    /// declines a step is not an error here; it is [`SecretPlacement::Refused`].
    pub fn place_secret(
        mut self,
        secret: &SecretFile,
    ) -> Result<(Self, SecretPlacement), ControlError> {
        if let Some(parent) = secret.parent() {
            let request = FileRequest::MakeDirectory {
                path: parent.into(),
                parents: true,
            };
            let (session, outcome) = self.file(request)?;
            self = session;
            if let Some(refusal) = refusal(&outcome, SecretStage::Directory) {
                return Ok((self, refusal));
            }
        }
        let create = FileRequest::Create {
            path: secret.path().into(),
            mode: PRIVATE_CREATE_MODE,
        };
        let (session, outcome) = self.file(create)?;
        self = session;
        if let Some(refusal) = refusal(&outcome, SecretStage::Create) {
            return Ok((self, refusal));
        }
        let value = secret.value();
        let (session, written) =
            self.write_whole_file(secret.path(), value.expose(), MAX_SECRET_BYTES)?;
        self = session;
        match written {
            WholeFileWrite::Written => {}
            WholeFileWrite::Failed(failure) => {
                return Ok((
                    self,
                    SecretPlacement::Refused {
                        stage: SecretStage::Write,
                        failure,
                    },
                ));
            }
            // A value is bounded below the transfer bound by its own constructor, so this arm
            // cannot be reached by any value that exists.
            WholeFileWrite::TooLarge => {
                return Ok((
                    self,
                    SecretPlacement::Refused {
                        stage: SecretStage::Write,
                        failure: FileFailure::Failed,
                    },
                ));
            }
        }
        let seal = FileRequest::SetMode {
            path: secret.path().into(),
            mode: secret.mode(),
        };
        let (session, outcome) = self.file(seal)?;
        self = session;
        if let Some(refusal) = refusal(&outcome, SecretStage::Mode) {
            return Ok((self, refusal));
        }
        Ok((self, SecretPlacement::Placed))
    }

    /// Places every secret in order, stopping at the first the guest declines.
    ///
    /// Stopping is deliberate. A launch that is missing one of its credentials is already a
    /// failed launch, and delivering the rest would put values into a sandbox that is about to
    /// be destroyed for want of the first.
    ///
    /// # Errors
    ///
    /// Returns a redacted File error after poisoning the transport exactly once.
    pub fn place_secrets(
        mut self,
        secrets: &[SecretFile],
    ) -> Result<(Self, SecretPlacement), ControlError> {
        for secret in secrets {
            let (session, placement) = self.place_secret(secret)?;
            self = session;
            if placement != SecretPlacement::Placed {
                return Ok((self, placement));
            }
        }
        Ok((self, SecretPlacement::Placed))
    }
}

/// Turns an outcome that is not a plain completion into the refusal that names its step.
///
/// An outcome of another shape is treated as a refusal rather than a protocol failure because
/// the answer was well formed and paired with its request; it simply did not say the step
/// happened, and a delivery that cannot prove it happened has not happened.
fn refusal(outcome: &FileOutcome, stage: SecretStage) -> Option<SecretPlacement> {
    match outcome {
        FileOutcome::Done => None,
        FileOutcome::Failed(failure) => Some(SecretPlacement::Refused {
            stage,
            failure: *failure,
        }),
        _ => Some(SecretPlacement::Refused {
            stage,
            failure: FileFailure::Failed,
        }),
    }
}
