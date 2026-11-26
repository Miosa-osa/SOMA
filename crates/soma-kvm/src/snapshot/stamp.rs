//! What the kernel says about one file's bytes without reading them.
//!
//! A file is identified by its device, inode, size, and modification and change times. The change
//! time moves on any write, resize, rename-over, or permission change and cannot be set from user
//! space, so a file whose stamp equals one recorded when its content was hashed holds those same
//! bytes. A stamp is trusted exactly as far as whoever recorded it: it says nothing about a file a
//! stranger could have written before the record was made.

use super::Digest;

/// The kernel's identity of one file's current content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileStamp {
    /// A stamp from its raw parts, as the kernel reported them.
    #[must_use]
    pub const fn new(
        device: u64,
        inode: u64,
        size: u64,
        modified: (i64, i64),
        changed: (i64, i64),
    ) -> Self {
        Self {
            device,
            inode,
            size,
            modified,
            changed,
        }
    }

    /// The stamp of an open file.
    #[cfg(unix)]
    #[must_use]
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self::new(
            metadata.dev(),
            metadata.ino(),
            metadata.size(),
            (metadata.mtime(), metadata.mtime_nsec()),
            (metadata.ctime(), metadata.ctime_nsec()),
        )
    }

    /// The length the file had when it was stamped.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// The change time, as seconds and nanoseconds since the epoch.
    #[must_use]
    pub const fn changed(&self) -> (i64, i64) {
        self.changed
    }

    /// The seven space-separated decimal fields of this stamp.
    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "{} {} {} {} {} {} {}",
            self.device,
            self.inode,
            self.size,
            self.modified.0,
            self.modified.1,
            self.changed.0,
            self.changed.1
        )
    }

    /// Reads the seven fields [`encode`](Self::encode) wrote, or `None` for anything else.
    pub fn decode<'a>(fields: &mut impl Iterator<Item = &'a str>) -> Option<Self> {
        let mut number = || fields.next()?.parse::<i64>().ok();
        let device = u64::try_from(number()?).ok()?;
        let inode = u64::try_from(number()?).ok()?;
        let size = u64::try_from(number()?).ok()?;
        let modified = (number()?, number()?);
        let changed = (number()?, number()?);
        Some(Self::new(device, inode, size, modified, changed))
    }
}

/// A trusted broker's record that one immutable artifact had this digest at this exact stamp.
///
/// Only the broker that owns the artifact's directory may produce one. A restore that receives
/// it may skip re-reading a file whose descriptor still has this stamp and whose record names
/// this digest; any other file is hashed as before.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactAttestation {
    digest: Digest,
    stamp: FileStamp,
}

impl ArtifactAttestation {
    /// Records that the bytes with this stamp hash to `digest`.
    #[must_use]
    pub const fn new(digest: Digest, stamp: FileStamp) -> Self {
        Self { digest, stamp }
    }

    #[must_use]
    pub const fn digest(&self) -> Digest {
        self.digest
    }

    #[must_use]
    pub const fn stamp(&self) -> FileStamp {
        self.stamp
    }

    /// The digest this attestation vouches for, when the file it describes is exactly `current`,
    /// `size` bytes long, and expected to hash to `expected`; `None` means the caller must hash.
    #[must_use]
    pub fn vouches(&self, current: FileStamp, size: u64, expected: Digest) -> Option<Digest> {
        (self.stamp == current && self.stamp.size == size && self.digest == expected)
            .then_some(self.digest)
    }

    /// One line: the digest in hexadecimal and the stamp fields.
    #[must_use]
    pub fn encode(&self) -> String {
        format!("{} {}", self.digest, self.stamp.encode())
    }

    /// Reads one line [`encode`](Self::encode) wrote, or `None` for anything else.
    #[must_use]
    pub fn decode(line: &str) -> Option<Self> {
        let mut fields = line.trim_end_matches('\n').split(' ');
        let digest = parse_digest(fields.next()?)?;
        let stamp = FileStamp::decode(&mut fields)?;
        fields.next().is_none().then_some(Self::new(digest, stamp))
    }
}

/// Which restore input an attestation describes.
///
/// The checkpoint memory and the Generation root are immutable after publication. A private head
/// is listed only for the head the broker has just cloned from an attested saved overlay and
/// stamped before any descriptor left it; any other head is hashed. A durable volume is writable
/// by its guest, so a stamp says nothing about its content and it is always hashed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttestedArtifact {
    Memory,
    Root,
    Head,
}

impl AttestedArtifact {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Root => "root",
            Self::Head => "head",
        }
    }

    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "memory" => Some(Self::Memory),
            "root" => Some(Self::Root),
            "head" => Some(Self::Head),
            _ => None,
        }
    }
}

/// The attestations one resident restore received, each optional.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RestoreAttestations {
    pub memory: Option<ArtifactAttestation>,
    pub root: Option<ArtifactAttestation>,
    pub head: Option<ArtifactAttestation>,
}

impl RestoreAttestations {
    /// Holds `attestation` for `artifact`, replacing any earlier one.
    pub const fn set(&mut self, artifact: AttestedArtifact, attestation: ArtifactAttestation) {
        match artifact {
            AttestedArtifact::Memory => self.memory = Some(attestation),
            AttestedArtifact::Root => self.root = Some(attestation),
            AttestedArtifact::Head => self.head = Some(attestation),
        }
    }
}

fn parse_digest(text: &str) -> Option<Digest> {
    if text.len() != Digest::LEN * 2 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut bytes = [0_u8; Digest::LEN];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(Digest::from_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::{ArtifactAttestation, Digest, FileStamp};

    fn attestation() -> ArtifactAttestation {
        ArtifactAttestation::new(
            Digest::of(b"memory"),
            FileStamp::new(66, 7, 4096, (10, 20), (30, 40)),
        )
    }

    #[test]
    fn a_durable_volume_can_never_be_named_for_attestation() {
        use super::{AttestedArtifact, RestoreAttestations};
        for token in ["memory", "root", "head"] {
            assert_eq!(
                AttestedArtifact::from_token(token).map(AttestedArtifact::token),
                Some(token)
            );
        }
        for token in ["volume", "workspace", "overlay", ""] {
            assert_eq!(AttestedArtifact::from_token(token), None, "{token}");
        }
        // The restore attestations have no slot a volume could occupy.
        let RestoreAttestations { memory, root, head } = RestoreAttestations::default();
        assert_eq!((memory, root, head), (None, None, None));
    }

    #[test]
    fn an_attestation_round_trips_through_its_line() {
        let line = attestation().encode();
        assert_eq!(ArtifactAttestation::decode(&line), Some(attestation()));
        assert_eq!(
            ArtifactAttestation::decode(&format!("{line}\n")),
            Some(attestation())
        );
    }

    #[test]
    fn a_damaged_line_is_no_attestation() {
        let line = attestation().encode();
        for damaged in [
            String::new(),
            format!("{line} 9"),
            line.replacen(' ', "  ", 1),
            line[1..].to_owned(),
            line.replace(" 4096 ", " -1 "),
            format!("zz{}", &line[2..]),
        ] {
            assert_eq!(ArtifactAttestation::decode(&damaged), None, "{damaged}");
        }
    }

    #[test]
    fn an_attestation_vouches_only_for_its_exact_stamp_size_and_digest() {
        let held = attestation();
        let digest = Digest::of(b"memory");
        let stamp = held.stamp();
        assert_eq!(held.vouches(stamp, 4096, digest), Some(digest));
        let touched = FileStamp::new(66, 7, 4096, (10, 20), (30, 41));
        assert_eq!(held.vouches(touched, 4096, digest), None);
        assert_eq!(held.vouches(stamp, 4097, digest), None);
        assert_eq!(held.vouches(stamp, 4096, Digest::of(b"other")), None);
    }
}
