use core::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::{
    Error, InitiatorAwaitingResponse, InitiatorHandshake, InstancePsk, ResponderHandshake,
    ResponderPendingResponse, ResponderPrivateKey, ResponderPublicKey, SessionBinding, resolver,
};

use self::wire::ENTROPY_SIZE;

mod wire;

/// Exact size of the one-launch bearer-secret page.
pub const LAUNCH_PAGE_SIZE: usize = 4096;

const RANDOM_SIZE: usize = 32 + 32 + ENTROPY_SIZE;
const RANDOM_ATTEMPTS: usize = 4;

/// Host-owned fresh launch secrets for one concrete Instance.
///
/// This state is neither cloneable nor accidentally reusable within the owned safe API.
///
/// ```compile_fail
/// use soma_guest::HostLaunchMaterial;
///
/// fn requires_clone<T: Clone>(_: &T) {}
///
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16]).unwrap();
/// requires_clone(&material);
/// ```
///
/// ```compile_fail
/// use soma_guest::HostLaunchMaterial;
///
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16]).unwrap();
/// let _delivered = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// let _reused = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// ```
pub struct HostLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
}

/// Host launch secrets after one delivery callback reported success.
///
/// Connecting a host owner consumes this state, so one reported delivery enables one attempt.
///
/// ```compile_fail
/// use soma_guest::{HostLaunchMaterial, ResponderKeypair};
///
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16]).unwrap();
/// let delivered = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// let responder = ResponderKeypair::generate().unwrap();
/// let _started = delivered.start_initiator(responder.public_key());
/// let _reused = delivered.binding();
/// ```
pub struct DeliveredHostLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
}

/// Guest-owned launch secrets removed from one non-snapshot page.
pub struct GuestLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
}

/// Guest session material available only after the caller repairs entropy.
///
/// Connecting a guest owner consumes this state, so one injected PSK authorizes one handshake.
///
/// ```compile_fail
/// use soma_guest::{GuestLaunchMaterial, HostLaunchMaterial, LAUNCH_PAGE_SIZE, ResponderKeypair};
///
/// let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16]).unwrap();
/// let mut page = [0; LAUNCH_PAGE_SIZE];
/// let host = host.deliver_with(|bytes| { page.copy_from_slice(bytes); Ok::<(), ()>(()) }).unwrap();
/// let guest = GuestLaunchMaterial::take_from_page(&mut page).unwrap();
/// let guest = guest.reseed_with(|_| Ok::<(), ()>(())).unwrap();
/// let responder = ResponderKeypair::generate().unwrap();
/// let (_, first) = host.start_initiator(responder.public_key()).unwrap();
/// let _pending = guest.start_responder(responder.private_key(), &first);
/// let _reused = guest.start_responder(responder.private_key(), &first);
/// ```
pub struct GuestSessionMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
}

impl HostLaunchMaterial {
    /// Generates fresh launch nonce, Instance PSK, and entropy from the operating system.
    ///
    /// # Errors
    ///
    /// Returns an error for zero identity fields or unavailable operating-system randomness.
    pub fn generate(
        generation: [u8; 32],
        instance: [u8; 16],
        operation: [u8; 16],
    ) -> Result<Self, Error> {
        Self::generate_with(generation, instance, operation, resolver::fill_os_random)
    }

    fn generate_with(
        generation: [u8; 32],
        instance: [u8; 16],
        operation: [u8; 16],
        mut fill: impl FnMut(&mut [u8]) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        validate_identity(generation, instance, operation)?;
        for _ in 0..RANDOM_ATTEMPTS {
            let mut random = Zeroizing::new([0_u8; RANDOM_SIZE]);
            fill(random.as_mut())?;
            let launch_nonce = array(&random[..], 0)?;
            let psk = secret_array(&random[..], 32)?;
            let entropy = secret_array(&random[..], 64)?;
            if launch_nonce != [0; 32]
                && psk.iter().any(|byte| *byte != 0)
                && entropy.iter().any(|byte| *byte != 0)
            {
                let binding = SessionBinding::new(generation, instance, operation, launch_nonce)?;
                return Ok(Self {
                    binding,
                    psk: InstancePsk::from_zeroizing(instance, psk)?,
                    entropy,
                });
            }
        }
        Err(Error::RandomnessUnavailable)
    }

    /// Borrows the exact transcript binding generated for this launch.
    #[must_use]
    pub const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Delivers one internally owned canonical launch page through a scoped callback.
    ///
    /// The internal page is zeroized after the callback returns on both success and failure.
    /// A callback that copies the bearer-secret bytes owns and must erase every resulting copy.
    ///
    /// # Errors
    ///
    /// Returns the callback error without yielding handshake-capable host material.
    pub fn deliver_with<E>(
        self,
        deliver: impl FnOnce(&[u8; LAUNCH_PAGE_SIZE]) -> Result<(), E>,
    ) -> Result<DeliveredHostLaunchMaterial, E> {
        let mut page = Zeroizing::new([0_u8; LAUNCH_PAGE_SIZE]);
        wire::encode(&mut page, &self.binding, self.psk.as_bytes(), &self.entropy);
        deliver(&page)?;
        Ok(DeliveredHostLaunchMaterial {
            binding: self.binding,
            psk: self.psk,
        })
    }
}

impl DeliveredHostLaunchMaterial {
    /// Borrows the exact transcript binding delivered to the guest.
    #[must_use]
    pub const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Consumes delivered material to start exactly one authenticated host handshake.
    ///
    /// # Errors
    ///
    /// Returns a redacted cryptographic setup error.
    pub(crate) fn start_initiator(
        self,
        responder: &ResponderPublicKey,
    ) -> Result<(InitiatorAwaitingResponse, Vec<u8>), Error> {
        InitiatorHandshake::start(&self.binding, responder, self.psk)
    }
}

impl GuestLaunchMaterial {
    /// Parses one exact launch page and wipes the entire supplied slice on every outcome.
    ///
    /// ```compile_fail
    /// use soma_guest::GuestLaunchMaterial;
    ///
    /// let mut page = vec![0_u8; 4096];
    /// let _ = GuestLaunchMaterial::take_from_page(&mut page);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::LaunchPageRejected`] for every malformed page.
    pub fn take_from_page(page: &mut [u8; LAUNCH_PAGE_SIZE]) -> Result<Self, Error> {
        let decoded = wire::decode(page);
        page.zeroize();
        decoded.map(|decoded| Self {
            binding: decoded.binding,
            psk: decoded.psk,
            entropy: decoded.entropy,
        })
    }

    /// Borrows the transcript binding before entropy repair.
    #[must_use]
    pub const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Consumes the injected seed at the guest entropy-repair boundary.
    ///
    /// The seed is zeroized after the callback returns.
    /// Any copy retained by the callback is outside this crate's erasure boundary.
    ///
    /// # Errors
    ///
    /// Returns the callback's error without exposing handshake-capable material.
    pub fn reseed_with<E>(
        self,
        reseed: impl FnOnce(&[u8; ENTROPY_SIZE]) -> Result<(), E>,
    ) -> Result<GuestSessionMaterial, E> {
        reseed(&self.entropy)?;
        Ok(GuestSessionMaterial {
            binding: self.binding,
            psk: self.psk,
        })
    }
}

impl GuestSessionMaterial {
    pub(crate) const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Starts the authenticated guest handshake after entropy repair.
    ///
    /// # Errors
    ///
    /// Returns a redacted authentication or setup error.
    pub(crate) fn start_responder(
        self,
        private_key: &ResponderPrivateKey,
        first: &[u8],
    ) -> Result<ResponderPendingResponse, Error> {
        ResponderHandshake::accept(&self.binding, private_key, self.psk, first)
    }
}

fn array<const N: usize>(source: &[u8], start: usize) -> Result<[u8; N], Error> {
    source
        .get(start..start.checked_add(N).ok_or(Error::LaunchPageRejected)?)
        .ok_or(Error::LaunchPageRejected)?
        .try_into()
        .map_err(|_| Error::LaunchPageRejected)
}

fn secret_array<const N: usize>(source: &[u8], start: usize) -> Result<Zeroizing<[u8; N]>, Error> {
    let bytes = source
        .get(start..start.checked_add(N).ok_or(Error::LaunchPageRejected)?)
        .ok_or(Error::LaunchPageRejected)?;
    let mut secret = Zeroizing::new([0; N]);
    secret.copy_from_slice(bytes);
    Ok(secret)
}

fn validate_identity(
    generation: [u8; 32],
    instance: [u8; 16],
    operation: [u8; 16],
) -> Result<(), Error> {
    if generation == [0; 32] || instance == [0; 16] || operation == [0; 16] {
        return Err(Error::InvalidBinding);
    }
    Ok(())
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

redacted_debug!(HostLaunchMaterial, "HostLaunchMaterial");
redacted_debug!(DeliveredHostLaunchMaterial, "DeliveredHostLaunchMaterial");
redacted_debug!(GuestLaunchMaterial, "GuestLaunchMaterial");
redacted_debug!(GuestSessionMaterial, "GuestSessionMaterial");

#[cfg(test)]
mod tests;
