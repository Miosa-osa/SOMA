use core::fmt;

use snow::{Builder, HandshakeState, params::NoiseParams};

use crate::{
    AuthenticatedSession, Error, InstancePsk, NOISE_PATTERN, ResponderPrivateKey,
    ResponderPublicKey, SessionBinding, resolver,
};

const MAX_HANDSHAKE_MESSAGE: usize = 256;

/// Starts the host side of the fixed two-message authenticated handshake.
pub struct InitiatorHandshake;

/// Owns an initiator handshake after message one and before message two.
pub struct InitiatorAwaitingResponse(HandshakeState);

/// Accepts message one on the guest responder side.
pub struct ResponderHandshake;

/// Holds message two until the responder's transport writes it to the peer.
pub struct ResponderPendingResponse {
    state: HandshakeState,
    response: Vec<u8>,
}

impl InitiatorHandshake {
    /// Starts one handshake and returns its bounded, length-prefixed first message.
    ///
    /// # Errors
    ///
    /// Returns a redacted setup error when the fixed suite cannot be initialized.
    pub fn start(
        binding: &SessionBinding,
        responder: &ResponderPublicKey,
        psk: &InstancePsk,
    ) -> Result<(InitiatorAwaitingResponse, Vec<u8>), Error> {
        psk.require_instance(binding.instance())?;
        let prologue = binding.prologue();
        let mut state = builder(&prologue, psk)?
            .remote_public_key(responder.as_bytes())
            .map_err(|_| Error::CryptoSetup)?
            .build_initiator()
            .map_err(|_| Error::CryptoSetup)?;
        let first = write_empty_handshake(&mut state)?;
        Ok((InitiatorAwaitingResponse(state), frame(&first)?))
    }
}

impl InitiatorAwaitingResponse {
    /// Authenticates message two and enters encrypted transport mode.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AuthenticationFailed`] for any peer-controlled failure.
    pub fn finish(mut self, second: &[u8]) -> Result<AuthenticatedSession, Error> {
        let message = unframe(second).map_err(|_| Error::AuthenticationFailed)?;
        read_empty_handshake(&mut self.0, message)?;
        let transport = self
            .0
            .into_transport_mode()
            .map_err(|_| Error::AuthenticationFailed)?;
        Ok(AuthenticatedSession::new(transport))
    }
}

impl ResponderHandshake {
    /// Authenticates message one and returns transport state plus framed message two.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AuthenticationFailed`] for any peer-controlled failure.
    pub fn accept(
        binding: &SessionBinding,
        private_key: &ResponderPrivateKey,
        psk: &InstancePsk,
        first: &[u8],
    ) -> Result<ResponderPendingResponse, Error> {
        psk.require_instance(binding.instance())?;
        let message = unframe(first).map_err(|_| Error::AuthenticationFailed)?;
        let prologue = binding.prologue();
        let mut state = builder(&prologue, psk)?
            .local_private_key(private_key.as_bytes())
            .map_err(|_| Error::CryptoSetup)?
            .build_responder()
            .map_err(|_| Error::CryptoSetup)?;
        read_empty_handshake(&mut state, message)?;
        let second = write_empty_handshake(&mut state)?;
        Ok(ResponderPendingResponse {
            state,
            response: frame(&second)?,
        })
    }
}

impl ResponderPendingResponse {
    /// Borrows the exact bounded second message that must be sent before transition.
    #[must_use]
    pub fn response(&self) -> &[u8] {
        &self.response
    }

    /// Enters transport mode after the caller has sent [`Self::response`].
    ///
    /// # Errors
    ///
    /// Returns a redacted setup error if Snow rejects its completed state.
    pub fn finish(self) -> Result<AuthenticatedSession, Error> {
        let transport = self
            .state
            .into_transport_mode()
            .map_err(|_| Error::CryptoSetup)?;
        Ok(AuthenticatedSession::new(transport))
    }
}

fn builder<'a>(prologue: &'a [u8], psk: &'a InstancePsk) -> Result<Builder<'a>, Error> {
    let params: NoiseParams = NOISE_PATTERN.parse().map_err(|_| Error::CryptoSetup)?;
    resolver::noise_builder(params)
        .prologue(prologue)
        .and_then(|builder| builder.psk(0, psk.as_bytes()))
        .map_err(|_| Error::CryptoSetup)
}

fn write_empty_handshake(state: &mut HandshakeState) -> Result<Vec<u8>, Error> {
    let mut output = vec![0_u8; MAX_HANDSHAKE_MESSAGE];
    let written = state
        .write_message(&[], &mut output)
        .map_err(|_| Error::CryptoSetup)?;
    output.truncate(written);
    Ok(output)
}

fn read_empty_handshake(state: &mut HandshakeState, message: &[u8]) -> Result<(), Error> {
    let mut payload = [0_u8; 1];
    let read = state
        .read_message(message, &mut payload)
        .map_err(|_| Error::AuthenticationFailed)?;
    (read == 0).then_some(()).ok_or(Error::AuthenticationFailed)
}

fn frame(message: &[u8]) -> Result<Vec<u8>, Error> {
    if message.is_empty() || message.len() > MAX_HANDSHAKE_MESSAGE {
        return Err(Error::HandshakeRejected);
    }
    let length = u16::try_from(message.len()).map_err(|_| Error::HandshakeRejected)?;
    let mut framed = Vec::with_capacity(message.len() + 2);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(message);
    Ok(framed)
}

fn unframe(framed: &[u8]) -> Result<&[u8], Error> {
    let header: [u8; 2] = framed
        .get(..2)
        .ok_or(Error::HandshakeRejected)?
        .try_into()
        .map_err(|_| Error::HandshakeRejected)?;
    let length = usize::from(u16::from_be_bytes(header));
    if length == 0 || length > MAX_HANDSHAKE_MESSAGE || framed.len() != length + 2 {
        return Err(Error::HandshakeRejected);
    }
    Ok(&framed[2..])
}

impl fmt::Debug for InitiatorHandshake {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InitiatorHandshake")
    }
}

impl fmt::Debug for InitiatorAwaitingResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InitiatorAwaitingResponse { .. }")
    }
}

impl fmt::Debug for ResponderHandshake {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResponderHandshake")
    }
}

impl fmt::Debug for ResponderPendingResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResponderPendingResponse { .. }")
    }
}

#[cfg(test)]
mod tests;
