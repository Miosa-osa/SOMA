//! The Suspension of ADR 0034: one tenant's paused sandbox, as an object distinct from every
//! Generation snapshot.
//!
//! A Suspension is a per-Instance tenant object. It is written with its own magic, `SOMASUS`, so
//! the Generation restore path, which only reads `SOMASNP`, rejects it on the first eight bytes,
//! and this decoder rejects a Generation snapshot the same way. It has exactly three members, the
//! guest memory image, the private overlay head and the machine state, and a header binding the
//! Generation, the shape, the owning tenant, the host profile that wrote it, and its expiry.
//!
//! Every member is encrypted at rest under a per-Suspension data key (see [`seal`]); the header
//! carries each member's plaintext length and SHA-256, so a resume verifies what it decrypted
//! against what was written, and a Suspension whose bytes do not verify is destroyed, not resumed.
//!
//! Header layout (big-endian, no padding):
//!
//! ```text
//! magic "SOMASUS\0"        8
//! schema version           u16
//! suspension id            16
//! generation id            32
//! host profile digest      32
//! tenant                   u16 length, 1..=MAX_TENANT bytes of UTF-8
//! shape                    u16 vcpus, u32 memory MiB, u32 storage MiB
//! created, expires         u64 Unix seconds each, created < expires <= created + MAX_RETENTION
//! members                  3 x (u8 role, u64 plaintext bytes, 32 SHA-256), in role order
//! ```

pub mod seal;
pub mod store;
#[cfg(test)]
mod store_tests;
#[cfg(test)]
mod tests;

use std::{error::Error, fmt};

use crate::{GenerationId, HostProfileDigest, ids::SuspensionId};

pub const MAGIC: [u8; 8] = *b"SOMASUS\0";
pub const SCHEMA_VERSION: u16 = 1;
/// The longest tenant identity a header carries.
pub const MAX_TENANT: usize = 128;
/// No Suspension lives longer than this; a Template's retention is bounded by it.
pub const MAX_RETENTION_SECS: u64 = 30 * 24 * 60 * 60;

/// What a member of a Suspension is. The order is the order members appear in the header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Member {
    /// The guest memory image at the suspension point.
    Memory = 1,
    /// The private writable overlay head at the same point.
    Overlay = 2,
    /// vCPU state, device model and platform state, in the snapshot codec's order.
    State = 3,
}

pub const MEMBERS: [Member; 3] = [Member::Memory, Member::Overlay, Member::State];

/// One member as the header records it: its plaintext size and digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemberRecord {
    pub bytes: u64,
    pub sha256: [u8; 32],
}

/// The machine shape the Instance ran at, which a resume must reproduce exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Shape {
    pub vcpus: u16,
    pub memory_mib: u32,
    pub storage_mib: u32,
}

/// A validated Suspension header.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Header {
    pub id: SuspensionId,
    pub generation: GenerationId,
    pub host_profile: HostProfileDigest,
    pub tenant: String,
    pub shape: Shape,
    pub created_unix: u64,
    pub expires_unix: u64,
    /// In [`MEMBERS`] order.
    pub members: [MemberRecord; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HeaderError {
    Truncated,
    Trailing,
    BadMagic,
    UnsupportedSchema(u16),
    ZeroId(&'static str),
    Tenant,
    Shape,
    Lifetime,
    MemberOrder,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("suspension header is truncated"),
            Self::Trailing => formatter.write_str("suspension header has trailing bytes"),
            Self::BadMagic => formatter.write_str("not a suspension (magic is not SOMASUS)"),
            Self::UnsupportedSchema(found) => {
                write!(formatter, "unsupported suspension schema {found}")
            }
            Self::ZeroId(what) => write!(formatter, "suspension {what} is all zero"),
            Self::Tenant => {
                formatter.write_str("suspension tenant is empty, too long, or not UTF-8")
            }
            Self::Shape => formatter.write_str("suspension shape has a zero dimension"),
            Self::Lifetime => formatter.write_str(
                "suspension expiry is not after creation or exceeds the retention ceiling",
            ),
            Self::MemberOrder => {
                formatter.write_str("suspension members are missing or out of order")
            }
        }
    }
}

impl Error for HeaderError {}

impl Header {
    /// Checks every field a decoded header must satisfy; `encode` refuses what this refuses.
    ///
    /// # Errors
    ///
    /// Returns the first rule the header breaks.
    pub fn validate(&self) -> Result<(), HeaderError> {
        if self.tenant.is_empty() || self.tenant.len() > MAX_TENANT {
            return Err(HeaderError::Tenant);
        }
        let shape = self.shape;
        if shape.vcpus == 0 || shape.memory_mib == 0 {
            return Err(HeaderError::Shape);
        }
        if self.expires_unix <= self.created_unix
            || self.expires_unix - self.created_unix > MAX_RETENTION_SECS
        {
            return Err(HeaderError::Lifetime);
        }
        Ok(())
    }

    /// The header bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Header::validate`]'s refusal; an invalid header is never written.
    pub fn encode(&self) -> Result<Vec<u8>, HeaderError> {
        self.validate()?;
        let mut out = Vec::with_capacity(256 + self.tenant.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&SCHEMA_VERSION.to_be_bytes());
        out.extend_from_slice(self.id.as_bytes());
        out.extend_from_slice(self.generation.as_bytes());
        out.extend_from_slice(self.host_profile.as_bytes());
        // `validate` bounded the tenant to MAX_TENANT bytes.
        out.extend_from_slice(
            &u16::try_from(self.tenant.len())
                .map_err(|_| HeaderError::Tenant)?
                .to_be_bytes(),
        );
        out.extend_from_slice(self.tenant.as_bytes());
        out.extend_from_slice(&self.shape.vcpus.to_be_bytes());
        out.extend_from_slice(&self.shape.memory_mib.to_be_bytes());
        out.extend_from_slice(&self.shape.storage_mib.to_be_bytes());
        out.extend_from_slice(&self.created_unix.to_be_bytes());
        out.extend_from_slice(&self.expires_unix.to_be_bytes());
        for (member, record) in MEMBERS.iter().zip(&self.members) {
            out.push(*member as u8);
            out.extend_from_slice(&record.bytes.to_be_bytes());
            out.extend_from_slice(&record.sha256);
        }
        Ok(out)
    }

    /// Decodes and fully validates hostile header bytes.
    ///
    /// # Errors
    ///
    /// Returns a typed [`HeaderError`] for a wrong magic or schema, a zero identity, a bad tenant,
    /// shape or lifetime, members out of order, a truncation, or trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, HeaderError> {
        let mut reader = Reader(bytes);
        if reader.array::<8>()? != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        let schema = u16::from_be_bytes(reader.array()?);
        if schema != SCHEMA_VERSION {
            return Err(HeaderError::UnsupportedSchema(schema));
        }
        let id = SuspensionId::new(reader.array()?).map_err(|_| HeaderError::ZeroId("id"))?;
        let generation =
            GenerationId::new(reader.array()?).map_err(|_| HeaderError::ZeroId("generation"))?;
        let host_profile = HostProfileDigest::new(reader.array()?)
            .map_err(|_| HeaderError::ZeroId("host profile"))?;
        let tenant_len = usize::from(u16::from_be_bytes(reader.array()?));
        if tenant_len == 0 || tenant_len > MAX_TENANT {
            return Err(HeaderError::Tenant);
        }
        let tenant = String::from_utf8(reader.take(tenant_len)?.to_vec())
            .map_err(|_| HeaderError::Tenant)?;
        let shape = Shape {
            vcpus: u16::from_be_bytes(reader.array()?),
            memory_mib: u32::from_be_bytes(reader.array()?),
            storage_mib: u32::from_be_bytes(reader.array()?),
        };
        let created_unix = u64::from_be_bytes(reader.array()?);
        let expires_unix = u64::from_be_bytes(reader.array()?);
        let mut members = [MemberRecord {
            bytes: 0,
            sha256: [0; 32],
        }; 3];
        for (expected, slot) in MEMBERS.iter().zip(&mut members) {
            if reader.array::<1>()?[0] != *expected as u8 {
                return Err(HeaderError::MemberOrder);
            }
            *slot = MemberRecord {
                bytes: u64::from_be_bytes(reader.array()?),
                sha256: reader.array()?,
            };
        }
        if !reader.0.is_empty() {
            return Err(HeaderError::Trailing);
        }
        let header = Self {
            id,
            generation,
            host_profile,
            tenant,
            shape,
            created_unix,
            expires_unix,
            members,
        };
        header.validate()?;
        Ok(header)
    }

    /// Whether the Suspension has expired at `now_unix` and must be destroyed rather than resumed.
    #[must_use]
    pub const fn expired_at(&self, now_unix: u64) -> bool {
        now_unix >= self.expires_unix
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], HeaderError> {
        if self.0.len() < count {
            return Err(HeaderError::Truncated);
        }
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], HeaderError> {
        let mut out = [0_u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}
