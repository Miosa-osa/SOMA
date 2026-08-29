use core::fmt;

use snow::params::NoiseParams;
use zeroize::Zeroizing;

use crate::{Error, NOISE_PATTERN, resolver};

/// A fresh 256-bit secret scoped to one concrete Instance.
pub(crate) struct InstancePsk {
    instance: [u8; 16],
    secret: Zeroizing<[u8; 32]>,
}

impl InstancePsk {
    /// Provisions secret bytes for one exact canonical Instance identity.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidKeyMaterial`] if either value is all zero.
    #[cfg(test)]
    pub(crate) fn provision_for(instance: [u8; 16], secret: [u8; 32]) -> Result<Self, Error> {
        let instance = nonzero(instance)?;
        let secret = nonzero(secret)?;
        Ok(Self {
            instance,
            secret: Zeroizing::new(secret),
        })
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.secret
    }

    pub(crate) fn from_zeroizing(
        instance: [u8; 16],
        secret: Zeroizing<[u8; 32]>,
    ) -> Result<Self, Error> {
        let instance = nonzero(instance)?;
        if secret.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidKeyMaterial);
        }
        Ok(Self { instance, secret })
    }

    pub(crate) fn require_instance(&self, instance: &[u8; 16]) -> Result<(), Error> {
        (self.instance == *instance)
            .then_some(())
            .ok_or(Error::PskInstanceMismatch)
    }
}

/// The Generation-scoped responder's X25519 private key.
pub struct ResponderPrivateKey(Zeroizing<[u8; 32]>);

impl ResponderPrivateKey {
    /// Wraps private key bytes and zeroizes the owned copy on drop.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidKeyMaterial`] for an all-zero value.
    pub fn new(bytes: [u8; 32]) -> Result<Self, Error> {
        Self::from_owned(Zeroizing::new(bytes))
    }

    /// Exposes a borrowed secret only for an explicit Generation-provisioning operation.
    ///
    /// Any copy made by the callback is outside this crate's zeroization boundary.
    pub fn expose_for_provisioning<R>(&self, operation: impl FnOnce(&[u8; 32]) -> R) -> R {
        operation(&self.0)
    }

    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    fn from_owned(bytes: Zeroizing<[u8; 32]>) -> Result<Self, Error> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(Error::InvalidKeyMaterial);
        }
        Ok(Self(bytes))
    }
}

/// The pinned Generation-scoped responder's X25519 public key.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ResponderPublicKey([u8; 32]);

impl ResponderPublicKey {
    /// Validates a pinned public key.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidKeyMaterial`] for an all-zero value.
    pub fn new(bytes: [u8; 32]) -> Result<Self, Error> {
        let bytes = nonzero(bytes)?;
        resolver::validate_responder_public_key(&bytes)?;
        Ok(Self(bytes))
    }

    /// Returns the non-secret public key bytes for a trusted Generation manifest.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// A generated responder keypair for a trusted fixture or Generation builder.
pub struct ResponderKeypair {
    private: ResponderPrivateKey,
    public: ResponderPublicKey,
}

impl ResponderKeypair {
    /// Generates a responder keypair using Snow's operating-system RNG resolver.
    ///
    /// # Errors
    ///
    /// Returns a redacted setup error if the suite or RNG is unavailable.
    pub fn generate() -> Result<Self, Error> {
        let params: NoiseParams = NOISE_PATTERN.parse().map_err(|_| Error::CryptoSetup)?;
        let generated = resolver::noise_builder(params)
            .generate_keypair()
            .map_err(|_| Error::CryptoSetup)?;
        let private_source = Zeroizing::new(generated.private);
        let private_bytes: &[u8; 32] = private_source
            .as_slice()
            .try_into()
            .map_err(|_| Error::CryptoSetup)?;
        let mut private = Zeroizing::new([0_u8; 32]);
        private.copy_from_slice(private_bytes);
        let private = ResponderPrivateKey::from_owned(private)?;
        let public = generated
            .public
            .try_into()
            .map_err(|_| Error::CryptoSetup)?;
        Ok(Self {
            private,
            public: ResponderPublicKey::new(public)?,
        })
    }

    /// Borrows the private half without exposing its bytes.
    #[must_use]
    pub const fn private_key(&self) -> &ResponderPrivateKey {
        &self.private
    }

    /// Borrows the public half.
    #[must_use]
    pub const fn public_key(&self) -> &ResponderPublicKey {
        &self.public
    }
}

macro_rules! redacted_debug {
    ($type:ty, $name:literal) => {
        impl fmt::Debug for $type {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($name, "([REDACTED])"))
            }
        }
    };
}

redacted_debug!(InstancePsk, "InstancePsk");
redacted_debug!(ResponderPrivateKey, "ResponderPrivateKey");
redacted_debug!(ResponderPublicKey, "ResponderPublicKey");
redacted_debug!(ResponderKeypair, "ResponderKeypair");

fn nonzero<const N: usize>(bytes: [u8; N]) -> Result<[u8; N], Error> {
    (!bytes.iter().all(|byte| *byte == 0))
        .then_some(bytes)
        .ok_or(Error::InvalidKeyMaterial)
}

#[cfg(test)]
mod tests;
