use core::fmt;

use zeroize::{Zeroize, Zeroizing};

use crate::{Error, InstancePsk, SessionBinding, resolver};

use self::wire::ENTROPY_SIZE;

mod network;
mod session;
mod wire;

pub use network::LaunchNetwork;
pub use session::{DeliveredHostLaunchMaterial, GuestSessionMaterial};

/// Exact size of the one-launch bearer-secret page.
pub const LAUNCH_PAGE_SIZE: usize = 4096;

/// Fixed guest-physical address of the dedicated launch-page memory slot.
///
/// The address lies above the 3 GiB RAM ceiling and above the five fixed virtio-mmio pages of
/// machine contract v1, so it never overlaps RAM, MMIO, or any boot structure.
/// The VMM maps one fresh 4 KiB anonymous slot here after restore and before vCPU resume.
/// The trusted guest agent maps the same address through `/dev/mem`, consumes the page once,
/// overwrites it with zeroes, and the VMM retires the slot after observing host-side zeroes.
pub const LAUNCH_PAGE_GUEST_ADDRESS: u64 = 0xd010_0000;

const RANDOM_SIZE: usize = 32 + 32 + ENTROPY_SIZE;
const RANDOM_ATTEMPTS: usize = 4;

/// Host-owned fresh launch secrets for one concrete Instance.
///
/// This state is neither cloneable nor accidentally reusable within the owned safe API.
///
/// ```compile_fail
/// use soma_guest::{HostLaunchMaterial, LaunchNetwork};
///
/// fn requires_clone<T: Clone>(_: &T) {}
///
/// let network = LaunchNetwork::new(3, 1, [2, 0, 0, 0, 0, 1], [10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 1], 1).unwrap();
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).unwrap();
/// requires_clone(&material);
/// ```
///
/// ```compile_fail
/// use soma_guest::{HostLaunchMaterial, LaunchNetwork};
///
/// let network = LaunchNetwork::new(3, 1, [2, 0, 0, 0, 0, 1], [10, 0, 0, 2], 24, [10, 0, 0, 1], [10, 0, 0, 1], 1).unwrap();
/// let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], network).unwrap();
/// let _delivered = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// let _reused = material.deliver_with(|_| Ok::<(), ()>(())).unwrap();
/// ```
pub struct HostLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
    network: LaunchNetwork,
}

/// Guest-owned launch secrets removed from one non-snapshot page.
pub struct GuestLaunchMaterial {
    binding: SessionBinding,
    psk: InstancePsk,
    entropy: Zeroizing<[u8; ENTROPY_SIZE]>,
    network: LaunchNetwork,
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
        network: LaunchNetwork,
    ) -> Result<Self, Error> {
        Self::generate_with(
            generation,
            instance,
            operation,
            network,
            resolver::fill_os_random,
        )
    }

    fn generate_with(
        generation: [u8; 32],
        instance: [u8; 16],
        operation: [u8; 16],
        network: LaunchNetwork,
        mut fill: impl FnMut(&mut [u8]) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        validate_identity(generation, instance, operation)?;
        for _ in 0..RANDOM_ATTEMPTS {
            let mut random = Zeroizing::new([0_u8; RANDOM_SIZE]);
            fill(random.as_mut())?;
            let mut reader = wire::Reader::new(&random[..]);
            let launch_nonce: [u8; 32] = reader.array()?;
            let psk = reader.secret_array::<32>()?;
            let entropy = reader.secret_array::<ENTROPY_SIZE>()?;
            if launch_nonce != [0; 32]
                && psk.iter().any(|byte| *byte != 0)
                && entropy.iter().any(|byte| *byte != 0)
            {
                let binding = SessionBinding::new(generation, instance, operation, launch_nonce)?;
                return Ok(Self {
                    binding,
                    psk: InstancePsk::from_zeroizing(instance, psk)?,
                    entropy,
                    network,
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

    /// Returns the non-secret network identity delivered with this launch.
    #[must_use]
    pub const fn network(&self) -> LaunchNetwork {
        self.network
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
        wire::encode(
            &mut page,
            &self.binding,
            self.psk.as_bytes(),
            &self.entropy,
            self.network,
        );
        deliver(&page)?;
        Ok(DeliveredHostLaunchMaterial::new(self.binding, self.psk))
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
            network: decoded.network,
        })
    }

    /// Borrows the transcript binding before entropy repair.
    #[must_use]
    pub const fn binding(&self) -> &SessionBinding {
        &self.binding
    }

    /// Returns the non-secret network identity carried by the consumed page.
    #[must_use]
    pub const fn network(&self) -> LaunchNetwork {
        self.network
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
        Ok(GuestSessionMaterial::new(self.binding, self.psk))
    }
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
redacted_debug!(GuestLaunchMaterial, "GuestLaunchMaterial");

#[cfg(test)]
mod tests;
