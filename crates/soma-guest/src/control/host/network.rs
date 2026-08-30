use crate::{ActivationChallenge, ActivationReceipt, ActivationScope, Error};

use super::{HostControlIo, RepairedHostControl};

impl<I: HostControlIo> RepairedHostControl<I> {
    /// Mints the single-use capability that lets the broker activate one network assignment.
    ///
    /// This owner exists only after authenticated repair and the fixed readiness probe, so the
    /// receipt is the guest evidence the privileged broker requires before it enables
    /// forwarding.
    /// The receipt binds this session's Instance and Launch operation, the broker's assignment
    /// generation and admitted intent digest, and the live authenticated transcript, so no
    /// other assignment or session can consume it.
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
