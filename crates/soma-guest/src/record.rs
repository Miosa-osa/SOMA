use core::fmt;

use snow::TransportState;
use zeroize::Zeroizing;

use crate::Error;

const OUTER_HEADER: usize = 2;
const INNER_HEADER: usize = 8 + 2;
const AEAD_TAG: usize = 16;
const NOISE_MESSAGE_MAX: usize = 65_535;
pub(crate) const MAX_RECORD_CIPHERTEXT: usize = NOISE_MESSAGE_MAX;
pub(crate) const MIN_RECORD_CIPHERTEXT: usize = AEAD_TAG + INNER_HEADER;

/// Largest caller payload accepted by one encrypted record.
pub const MAX_RECORD_PAYLOAD: usize = NOISE_MESSAGE_MAX - AEAD_TAG - INNER_HEADER;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Role {
    Initiator,
    Responder,
}

/// One authenticated, ordered, bidirectional Noise transport session.
pub(crate) struct AuthenticatedSession {
    transport: TransportState,
    role: Role,
    transcript: [u8; 32],
    next_send: u64,
    next_receive: u64,
    poisoned: bool,
}

impl AuthenticatedSession {
    pub(crate) fn new(transport: TransportState, transcript: [u8; 32]) -> Self {
        let role = if transport.is_initiator() {
            Role::Initiator
        } else {
            Role::Responder
        };
        Self {
            transport,
            role,
            transcript,
            next_send: 0,
            next_receive: 0,
            poisoned: false,
        }
    }

    /// Returns the final Noise handshake hash both authenticated peers computed.
    pub(crate) const fn transcript(&self) -> &[u8; 32] {
        &self.transcript
    }

    /// Encrypts one caller payload into an exact length-prefixed record.
    ///
    /// # Errors
    ///
    /// Returns [`Error::RecordTooLarge`] for an oversized payload or
    /// [`Error::SessionExhausted`] when the directional sequence is exhausted.
    pub(crate) fn seal(&mut self, payload: &[u8]) -> Result<Vec<u8>, Error> {
        if self.poisoned {
            return Err(Error::SessionPoisoned);
        }
        if payload.len() > MAX_RECORD_PAYLOAD {
            return Err(Error::RecordTooLarge);
        }
        let following = self
            .next_send
            .checked_add(1)
            .ok_or(Error::SessionExhausted)?;
        let payload_length = u16::try_from(payload.len()).map_err(|_| Error::RecordTooLarge)?;
        let mut plaintext = Zeroizing::new(Vec::with_capacity(INNER_HEADER + payload.len()));
        plaintext.extend_from_slice(&self.next_send.to_be_bytes());
        plaintext.extend_from_slice(&payload_length.to_be_bytes());
        plaintext.extend_from_slice(payload);
        let mut ciphertext = vec![0_u8; plaintext.len() + AEAD_TAG];
        let written = self
            .transport
            .write_message(&plaintext, &mut ciphertext)
            .map_err(|_| Error::SessionExhausted)?;
        ciphertext.truncate(written);
        let length = u16::try_from(written).map_err(|_| Error::RecordTooLarge)?;
        let mut framed = Vec::with_capacity(OUTER_HEADER + written);
        framed.extend_from_slice(&length.to_be_bytes());
        framed.extend_from_slice(&ciphertext);
        self.next_send = following;
        Ok(framed)
    }

    /// Authenticates and decrypts one exact, in-order peer record.
    ///
    /// The first peer-controlled error permanently poisons this session.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PeerRecordRejected`] for the first invalid peer record and
    /// [`Error::SessionPoisoned`] for every later attempt.
    pub(crate) fn open(&mut self, framed: &[u8]) -> Result<Vec<u8>, Error> {
        if self.poisoned {
            return Err(Error::SessionPoisoned);
        }
        if let Ok(payload) = self.open_once(framed) {
            Ok(payload)
        } else {
            self.poisoned = true;
            Err(Error::PeerRecordRejected)
        }
    }

    fn open_once(&mut self, framed: &[u8]) -> Result<Vec<u8>, ()> {
        let ciphertext = exact_ciphertext(framed)?;
        let length = ciphertext.len();
        let mut plaintext = Zeroizing::new(vec![0_u8; length - AEAD_TAG]);
        let read = self
            .transport
            .read_message(ciphertext, &mut plaintext)
            .map_err(|_| ())?;
        plaintext.truncate(read);
        let payload = exact_plaintext(self.next_receive, &plaintext)?;
        self.next_receive = self.next_receive.checked_add(1).ok_or(())?;
        Ok(payload.to_vec())
    }
}

fn exact_ciphertext(framed: &[u8]) -> Result<&[u8], ()> {
    let header: [u8; 2] = framed
        .get(..OUTER_HEADER)
        .ok_or(())?
        .try_into()
        .map_err(|_| ())?;
    let length = usize::from(u16::from_be_bytes(header));
    if length < MIN_RECORD_CIPHERTEXT || framed.len() != OUTER_HEADER + length {
        return Err(());
    }
    Ok(&framed[OUTER_HEADER..])
}

fn exact_plaintext(expected_sequence: u64, plaintext: &[u8]) -> Result<&[u8], ()> {
    let sequence = u64::from_be_bytes(plaintext.get(..8).ok_or(())?.try_into().map_err(|_| ())?);
    let payload_length = usize::from(u16::from_be_bytes(
        plaintext
            .get(8..INNER_HEADER)
            .ok_or(())?
            .try_into()
            .map_err(|_| ())?,
    ));
    if sequence != expected_sequence || plaintext.len() != INNER_HEADER + payload_length {
        return Err(());
    }
    Ok(&plaintext[INNER_HEADER..])
}

impl fmt::Debug for AuthenticatedSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedSession")
            .field("role", &self.role)
            .field("poisoned", &self.poisoned)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
