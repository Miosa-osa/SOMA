//! The durable sidecar that describes one sandbox volume beside its backing file.
//!
//! The sidecar is written before the state it describes is acted on, and it is the record the
//! host rebuilds from after a reboot, when the `tmpfs` Instance registry is empty.
//! The encoding is fixed-layout and checked the way every other persisted SOMA artifact is: a
//! bad magic, an unknown version, a wrong length, a corrupt checksum, an unknown state, an
//! all-zero identity, or a tenant that is empty or too long is refused rather than repaired.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{GenerationId, InstanceId, VolumeId};

/// Magic of one volume sidecar.
pub const SIDECAR_MAGIC: &[u8; 8] = b"SOMAVOL\0";
/// The sidecar schema version.
pub const SIDECAR_VERSION: u16 = 1;
/// The maximum tenant label a sidecar accepts.
pub const MAX_TENANT_BYTES: usize = 128;

/// The fixed encoded size of one sidecar.
pub const SIDECAR_LEN: usize =
    8 + 2 + 1 + 16 + 16 + 32 + 2 + MAX_TENANT_BYTES + 4 + 16 + 8 + 8 + 32;

/// The persistent path set a Step A volume carries, in the manifest's order.
///
/// Nothing outside this set is persistent in Step A: the root file system, `/etc`, package
/// state, and `/usr` are not kept, and a caller that needs them waits for ADR 0053 Step B.
pub const PERSISTENCE_PATHS: [&str; 4] = [
    "/home/sandbox",
    "/workspace",
    "/var/lib/docker",
    "/opt/user",
];

/// What a sidecar says about a volume's attachment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum VolumeState {
    /// A machine holds the volume; the file is open.
    Attached = 1,
    /// No machine holds the volume; the file is closed and the data is safe.
    Detached = 2,
    /// The volume is being destroyed; the decision is made and the reaper finishes it.
    Deleting = 3,
}

impl VolumeState {
    /// Decodes one state code, or `None` when this build does not know it.
    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Attached,
            2 => Self::Detached,
            3 => Self::Deleting,
            _ => return None,
        })
    }

    /// Returns the stable wire code.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }
}

/// Why a sidecar was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SidecarError {
    /// The byte length is not exactly [`SIDECAR_LEN`].
    Length,
    /// The magic does not match.
    Magic,
    /// The schema version is not this build's.
    Version,
    /// The trailing checksum does not match the body.
    Checksum,
    /// The state code is unknown.
    State,
    /// The tenant is empty or longer than [`MAX_TENANT_BYTES`].
    Tenant,
    /// An identity is malformed, such as the all-zero value.
    Identity,
}

impl fmt::Display for SidecarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length => formatter.write_str("volume sidecar has the wrong length"),
            Self::Magic => formatter.write_str("volume sidecar magic does not match"),
            Self::Version => formatter.write_str("volume sidecar version is not supported"),
            Self::Checksum => formatter.write_str("volume sidecar checksum does not match"),
            Self::State => formatter.write_str("volume sidecar state is unknown"),
            Self::Tenant => formatter.write_str("volume sidecar tenant is empty or too long"),
            Self::Identity => formatter.write_str("volume sidecar identity is malformed"),
        }
    }
}

impl std::error::Error for SidecarError {}

/// The durable description of one volume.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeSidecar {
    /// The volume this sidecar describes.
    pub volume: VolumeId,
    /// The Instance that owns it.
    pub instance: InstanceId,
    /// The Generation the owning Instance was launched from.
    pub generation: GenerationId,
    /// The owning tenant label.
    pub tenant: String,
    /// The disk class in GiB, one of the platform classes.
    pub size_class_gib: u32,
    /// The current attachment state.
    pub state: VolumeState,
    /// An opaque identifier of the per-volume data key; never key material.
    pub dek_id: [u8; 16],
    /// Unix nanoseconds when the volume was created.
    pub created_at_nanos: u64,
    /// Unix nanoseconds of the last recorded transition.
    pub last_transition_nanos: u64,
}

impl VolumeSidecar {
    /// Encodes the sidecar with its trailing checksum.
    ///
    /// # Errors
    ///
    /// Returns [`SidecarError::Tenant`] when the tenant is empty or longer than
    /// [`MAX_TENANT_BYTES`] bytes.
    pub fn encode(&self) -> Result<[u8; SIDECAR_LEN], SidecarError> {
        if self.tenant.is_empty() || self.tenant.len() > MAX_TENANT_BYTES {
            return Err(SidecarError::Tenant);
        }
        let tenant = self.tenant.as_bytes();
        let tenant_len = u16::try_from(tenant.len()).map_err(|_| SidecarError::Tenant)?;
        let mut tenant_field = [0_u8; MAX_TENANT_BYTES];
        tenant_field[..tenant.len()].copy_from_slice(tenant);
        let mut out = [0_u8; SIDECAR_LEN];
        let mut cursor = 0;
        let mut put = |bytes: &[u8]| {
            out[cursor..cursor + bytes.len()].copy_from_slice(bytes);
            cursor += bytes.len();
        };
        put(SIDECAR_MAGIC);
        put(&SIDECAR_VERSION.to_be_bytes());
        put(&[self.state.code()]);
        put(self.volume.as_bytes());
        put(self.instance.as_bytes());
        put(self.generation.as_bytes());
        put(&tenant_len.to_be_bytes());
        put(&tenant_field);
        put(&self.size_class_gib.to_be_bytes());
        put(&self.dek_id);
        put(&self.created_at_nanos.to_be_bytes());
        put(&self.last_transition_nanos.to_be_bytes());
        let body_len = SIDECAR_LEN - 32;
        let checksum: [u8; 32] = Sha256::digest(&out[..body_len]).into();
        out[body_len..].copy_from_slice(&checksum);
        Ok(out)
    }

    /// Decodes one exact sidecar.
    ///
    /// # Errors
    ///
    /// Returns a [`SidecarError`] for any malformed, short, long, unknown, or mismatched input.
    pub fn decode(bytes: &[u8]) -> Result<Self, SidecarError> {
        if bytes.len() != SIDECAR_LEN {
            return Err(SidecarError::Length);
        }
        if &bytes[..8] != SIDECAR_MAGIC {
            return Err(SidecarError::Magic);
        }
        let body_len = SIDECAR_LEN - 32;
        let checksum: [u8; 32] = Sha256::digest(&bytes[..body_len]).into();
        if checksum != bytes[body_len..] {
            return Err(SidecarError::Checksum);
        }
        let mut cursor = 8;
        let mut take = |n: usize| {
            let slice = &bytes[cursor..cursor + n];
            cursor += n;
            slice
        };
        if u16::from_be_bytes(array(take(2))) != SIDECAR_VERSION {
            return Err(SidecarError::Version);
        }
        let state = VolumeState::from_code(take(1)[0]).ok_or(SidecarError::State)?;
        let volume = VolumeId::new(array(take(16))).map_err(|_| SidecarError::Identity)?;
        let instance = InstanceId::new(array(take(16))).map_err(|_| SidecarError::Identity)?;
        let generation = GenerationId::new(array(take(32))).map_err(|_| SidecarError::Identity)?;
        let tenant_len = usize::from(u16::from_be_bytes(array(take(2))));
        let tenant_field = take(MAX_TENANT_BYTES);
        if tenant_len == 0 || tenant_len > MAX_TENANT_BYTES {
            return Err(SidecarError::Tenant);
        }
        let tenant = std::str::from_utf8(&tenant_field[..tenant_len])
            .map_err(|_| SidecarError::Tenant)?
            .to_owned();
        if tenant_field[tenant_len..].iter().any(|byte| *byte != 0) {
            return Err(SidecarError::Tenant);
        }
        let size_class_gib = u32::from_be_bytes(array(take(4)));
        let dek_id = array(take(16));
        let created_at_nanos = u64::from_be_bytes(array(take(8)));
        let last_transition_nanos = u64::from_be_bytes(array(take(8)));
        Ok(Self {
            volume,
            instance,
            generation,
            tenant,
            size_class_gib,
            state,
            dek_id,
            created_at_nanos,
            last_transition_nanos,
        })
    }
}

fn array<const N: usize>(slice: &[u8]) -> [u8; N] {
    let mut out = [0; N];
    out.copy_from_slice(slice);
    out
}
#[cfg(test)]
mod tests;
