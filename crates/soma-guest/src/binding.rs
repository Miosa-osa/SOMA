use core::fmt;

use crate::Error;

const DOMAIN: &[u8] = b"SOMA-GUEST-CONTROL\0";
const SCHEMA_VERSION: u16 = 1;
pub(crate) const AUTH_PROFILE: u16 = 1;
pub(crate) const PROLOGUE_LEN: usize = DOMAIN.len() + 2 + 2 + 32 + 16 + 16 + 32;

/// Canonical context bound into the Noise handshake transcript.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct SessionBinding {
    generation: [u8; 32],
    instance: [u8; 16],
    operation: [u8; 16],
    launch_nonce: [u8; 32],
}

impl SessionBinding {
    /// Creates a binding from raw, canonical identity bytes and a fresh launch nonce.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidBinding`] if any field is entirely zero.
    pub fn new(
        generation: [u8; 32],
        instance: [u8; 16],
        operation: [u8; 16],
        launch_nonce: [u8; 32],
    ) -> Result<Self, Error> {
        if generation.iter().all(|byte| *byte == 0)
            || instance.iter().all(|byte| *byte == 0)
            || operation.iter().all(|byte| *byte == 0)
            || launch_nonce.iter().all(|byte| *byte == 0)
        {
            return Err(Error::InvalidBinding);
        }
        Ok(Self {
            generation,
            instance,
            operation,
            launch_nonce,
        })
    }

    pub(crate) fn prologue(&self) -> [u8; PROLOGUE_LEN] {
        let mut encoded = [0_u8; PROLOGUE_LEN];
        let mut cursor = 0;
        copy(&mut encoded, &mut cursor, DOMAIN);
        copy(&mut encoded, &mut cursor, &SCHEMA_VERSION.to_be_bytes());
        copy(&mut encoded, &mut cursor, &AUTH_PROFILE.to_be_bytes());
        copy(&mut encoded, &mut cursor, &self.generation);
        copy(&mut encoded, &mut cursor, &self.instance);
        copy(&mut encoded, &mut cursor, &self.operation);
        copy(&mut encoded, &mut cursor, &self.launch_nonce);
        debug_assert_eq!(cursor, PROLOGUE_LEN);
        encoded
    }

    /// Returns the non-secret canonical Instance identity bound into the transcript.
    #[must_use]
    pub const fn instance(&self) -> &[u8; 16] {
        &self.instance
    }

    /// Returns the non-secret Generation digest bound into the transcript.
    #[must_use]
    pub const fn generation(&self) -> &[u8; 32] {
        &self.generation
    }

    /// Returns the non-secret Launch operation identity bound into the transcript.
    #[must_use]
    pub const fn operation(&self) -> &[u8; 16] {
        &self.operation
    }

    pub(crate) const fn launch_nonce(&self) -> &[u8; 32] {
        &self.launch_nonce
    }
}

impl fmt::Debug for SessionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionBinding { .. }")
    }
}

fn copy(destination: &mut [u8], cursor: &mut usize, source: &[u8]) {
    let end = *cursor + source.len();
    destination[*cursor..end].copy_from_slice(source);
    *cursor = end;
}

#[cfg(test)]
mod tests;
