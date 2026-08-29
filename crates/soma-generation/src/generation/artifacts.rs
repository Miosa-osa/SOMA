use std::fmt;

use sha2::{Digest as _, Sha256};
use soma::OciDigest;

/// The typed role of one immutable Generation artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ArtifactRole {
    /// Uncompressed `x86_64` ELF kernel with the PVH entry note.
    Kernel,
    /// Deterministic `newc` initramfs carrying early init and the guest agent.
    Initramfs,
    /// Immutable read-only EROFS root filesystem.
    ErofsRoot,
    /// Sterile ext4 overlay template for one writable size class.
    OverlayTemplate,
    /// Statically linked guest agent executable.
    GuestAgent,
    /// Statically linked early init executable.
    EarlyInit,
    /// Certified guest memory snapshot.
    MemorySnapshot,
    /// Canonical snapshot state manifest.
    StateManifest,
    /// Canonical `SOMAGEN` Generation manifest of a certified, ready Generation.
    GenerationManifest,
    /// Canonical `SOMACAN` manifest of a Generation Candidate, which is never launchable.
    GenerationCandidate,
}

impl ArtifactRole {
    /// Returns the fixed media type bound to this role.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Kernel => "application/vnd.soma.kernel.elf.v1",
            Self::Initramfs => "application/vnd.soma.initramfs.newc.v1",
            Self::ErofsRoot => "application/vnd.soma.rootfs.erofs.v1",
            Self::OverlayTemplate => "application/vnd.soma.overlay-template.ext4.v1",
            Self::GuestAgent => "application/vnd.soma.guest-agent.elf.v1",
            Self::EarlyInit => "application/vnd.soma.early-init.elf.v1",
            Self::MemorySnapshot => "application/vnd.soma.snapshot.memory.v1",
            Self::StateManifest => "application/vnd.soma.snapshot.state.v1",
            Self::GenerationManifest => "application/vnd.soma.generation.v1",
            Self::GenerationCandidate => "application/vnd.soma.generation-candidate.v1",
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Kernel => 1,
            Self::Initramfs => 2,
            Self::ErofsRoot => 3,
            Self::OverlayTemplate => 4,
            Self::GuestAgent => 5,
            Self::EarlyInit => 6,
            Self::MemorySnapshot => 7,
            Self::StateManifest => 8,
            Self::GenerationManifest => 9,
            Self::GenerationCandidate => 10,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Kernel,
            2 => Self::Initramfs,
            3 => Self::ErofsRoot,
            4 => Self::OverlayTemplate,
            5 => Self::GuestAgent,
            6 => Self::EarlyInit,
            7 => Self::MemorySnapshot,
            8 => Self::StateManifest,
            9 => Self::GenerationManifest,
            10 => Self::GenerationCandidate,
            _ => return None,
        })
    }
}

/// One raw SHA-256 digest used inside binary Generation artifacts.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Hashes bytes that are fully present in memory.
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        let output = Sha256::digest(bytes);
        let mut value = [0_u8; 32];
        value.copy_from_slice(output.as_ref());
        Self(value)
    }

    /// Wraps raw digest bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Converts a canonical OCI digest string into raw bytes.
    ///
    /// # Panics
    ///
    /// Cannot panic for an [`OciDigest`], whose constructor already enforces the exact
    /// `sha256:` plus 64 lowercase hex form.
    #[must_use]
    pub fn from_oci(digest: &OciDigest) -> Self {
        let hex = digest
            .as_str()
            .strip_prefix("sha256:")
            .expect("OciDigest always has a sha256 prefix");
        let mut value = [0_u8; 32];
        for (index, pair) in hex.as_bytes().as_chunks::<2>().0.iter().enumerate() {
            value[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        Self(value)
    }

    /// Converts the raw bytes into the canonical OCI digest string form.
    ///
    /// # Panics
    ///
    /// Cannot panic because the lowercase hex rendering is always a canonical digest.
    #[must_use]
    pub fn to_oci(&self) -> OciDigest {
        OciDigest::parse(self.to_string()).expect("hex output is a canonical OCI digest")
    }
}

const fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Sha256Digest({self})")
    }
}

/// One immutable artifact reference with a typed role, media type, digest, and exact size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactDescriptor {
    /// The typed role of the artifact.
    pub role: ArtifactRole,
    /// The SHA-256 digest of the exact artifact bytes.
    pub digest: Sha256Digest,
    /// The exact artifact byte length.
    pub size: u64,
}

impl ArtifactDescriptor {
    /// Returns the media type fixed by the artifact role.
    #[must_use]
    pub const fn media_type(&self) -> &'static str {
        self.role.media_type()
    }

    pub(crate) fn to_store_descriptor(self) -> crate::oci::Descriptor {
        crate::oci::Descriptor {
            media_type: self.media_type().to_owned(),
            digest: self.digest.to_oci(),
            size: self.size,
            platform: None,
        }
    }
}
