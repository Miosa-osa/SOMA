use crate::{ActivationChallenge, ActivationReceipt, ActivationScope, Error};

use super::{HostControlIo, RepairedHostControl};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Returns this session's live authenticated handshake transcript.
    ///
    /// The value is not a secret; it already crosses the wire inside every activation receipt.
    /// It is published only on this owner, which exists solely after authenticated repair and
    /// the fixed readiness probe, so a caller that holds one holds proof that this exact
    /// session reached its terminal readiness result.
    #[must_use]
    pub fn session_transcript(&self) -> [u8; 32] {
        *self.channel.session.transcript()
    }

    /// Mints the single-use capability that lets the broker activate one network assignment.
    ///
    /// The receipt binds this session's Instance and Launch operation, the broker's assignment
    /// generation and admitted intent digest, and the live authenticated transcript, so no
    /// other assignment or session can consume it.
    ///
    /// That this owner exists only after authenticated repair is a fact about this process, not
    /// a property of the receipt: the receipt is keyed by the challenge the broker handed to the
    /// claiming peer, so the broker learns only that the presenter received that challenge.
    /// See [`crate::ActivationChallenge`] for exactly what the capability does and does not
    /// prove.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidActivationScope`] for a zero generation or intent digest and
    /// [`Error::CryptoSetup`] when the fixed cryptographic suite is unavailable.
    pub fn network_activation(
        &self,
        challenge: &ActivationChallenge,
        generation: u32,
        intent: [u8; 32],
    ) -> Result<ActivationReceipt, Error> {
        let scope = ActivationScope::new(
            *self.binding.instance(),
            *self.binding.operation(),
            generation,
            intent,
        )?;
        let transcript = *self.channel.session.transcript();
        let tag = challenge.tag(&scope, &transcript)?;
        Ok(ActivationReceipt::new(transcript, tag))
    }
}
