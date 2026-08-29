//! The descriptor manifest: the only resources the VMM receives, in a fixed slot order.
//!
//! Slots 0, 1, and 2 are always the null input stream and the log stream twice, because the
//! Rust runtime aborts at startup when a standard descriptor is closed and `/dev/null` cannot
//! be opened, which is exactly the situation inside an empty root.
//! Manifest roles occupy slots `3..3 + len`, the executable occupies the next slot until
//! `execveat` closes it, and nothing else may exist.

use std::{error::Error, fmt};

/// The three inherited standard descriptors.
pub const STANDARD_STREAMS: u32 = 3;
/// The first slot assigned to a manifest role.
pub const FIRST_MANIFEST_SLOT: u32 = STANDARD_STREAMS;
/// A bound that keeps the table well inside any `RLIMIT_NOFILE` the profile would accept.
const MAX_ROLES: usize = 60;

/// Immutable artifacts transferred by the broker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Kernel,
    Initramfs,
    MemorySnapshot,
    DeviceState,
}

/// The typed role of one transferred descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorRole {
    /// An open `/dev/kvm`; VM and vCPU descriptors are created inside the jail.
    Kvm,
    /// The transferred TAP endpoint; `TUNSETIFF` already happened on the broker side.
    Tap,
    RootDisk,
    OverlayHead,
    Artifact(ArtifactKind),
    /// The `SOCK_SEQPACKET` control socket.
    Control,
    Log,
    /// An eventfd slot for irqfd or ioeventfd wiring.
    Event,
}

/// The `st_mode` file type the verifier accepts for a role.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorKind {
    CharDevice,
    RegularFile,
    Socket,
    Fifo,
    /// Kernel anonymous inodes such as eventfd carry no file-type bits at all.
    AnonInode,
}

impl DescriptorRole {
    #[must_use]
    pub fn expected_kinds(self) -> &'static [DescriptorKind] {
        match self {
            Self::Kvm | Self::Tap => &[DescriptorKind::CharDevice],
            Self::RootDisk | Self::OverlayHead | Self::Artifact(_) => {
                &[DescriptorKind::RegularFile]
            }
            Self::Control => &[DescriptorKind::Socket],
            Self::Log => &[
                DescriptorKind::Socket,
                DescriptorKind::RegularFile,
                DescriptorKind::Fifo,
                DescriptorKind::CharDevice,
            ],
            Self::Event => &[DescriptorKind::AnonInode],
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Kvm => "kvm",
            Self::Tap => "tap",
            Self::RootDisk => "root-disk",
            Self::OverlayHead => "overlay-head",
            Self::Artifact(ArtifactKind::Kernel) => "artifact.kernel",
            Self::Artifact(ArtifactKind::Initramfs) => "artifact.initramfs",
            Self::Artifact(ArtifactKind::MemorySnapshot) => "artifact.memory-snapshot",
            Self::Artifact(ArtifactKind::DeviceState) => "artifact.device-state",
            Self::Control => "control",
            Self::Log => "log",
            Self::Event => "event",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        const ALL: [DescriptorRole; 11] = [
            DescriptorRole::Kvm,
            DescriptorRole::Tap,
            DescriptorRole::RootDisk,
            DescriptorRole::OverlayHead,
            DescriptorRole::Artifact(ArtifactKind::Kernel),
            DescriptorRole::Artifact(ArtifactKind::Initramfs),
            DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
            DescriptorRole::Artifact(ArtifactKind::DeviceState),
            DescriptorRole::Control,
            DescriptorRole::Log,
            DescriptorRole::Event,
        ];
        ALL.into_iter().find(|role| role.token() == token)
    }
}

/// Typed manifest rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Empty,
    TooLong { max: usize },
    MissingControl,
    DuplicateSingleton(DescriptorRole),
    UnknownToken,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(formatter, "manifest has no descriptor"),
            Self::TooLong { max } => write!(formatter, "manifest exceeds {max} descriptors"),
            Self::MissingControl => write!(formatter, "manifest has no control socket"),
            Self::DuplicateSingleton(role) => {
                write!(formatter, "manifest lists {role:?} more than once")
            }
            Self::UnknownToken => write!(formatter, "manifest encoding contains an unknown role"),
        }
    }
}

impl Error for ManifestError {}

/// An ordered, validated list of descriptor roles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DescriptorManifest {
    roles: Vec<DescriptorRole>,
}

impl DescriptorManifest {
    /// Validates the role list: non-empty, bounded, exactly one control socket, and at most one
    /// KVM and one TAP descriptor.
    ///
    /// # Errors
    ///
    /// Returns the first [`ManifestError`] found.
    pub fn new(roles: Vec<DescriptorRole>) -> Result<Self, ManifestError> {
        if roles.is_empty() {
            return Err(ManifestError::Empty);
        }
        if roles.len() > MAX_ROLES {
            return Err(ManifestError::TooLong { max: MAX_ROLES });
        }
        for singleton in [
            DescriptorRole::Control,
            DescriptorRole::Kvm,
            DescriptorRole::Tap,
        ] {
            if roles.iter().filter(|role| **role == singleton).count() > 1 {
                return Err(ManifestError::DuplicateSingleton(singleton));
            }
        }
        if !roles.contains(&DescriptorRole::Control) {
            return Err(ManifestError::MissingControl);
        }
        Ok(Self { roles })
    }

    #[must_use]
    pub fn roles(&self) -> &[DescriptorRole] {
        &self.roles
    }

    /// The slot number of the manifest entry at `index`.
    #[must_use]
    pub fn slot_of(&self, index: usize) -> u32 {
        FIRST_MANIFEST_SLOT + u32::try_from(index).unwrap_or(u32::MAX)
    }

    /// The slot that holds the executable until `execveat` closes it.
    #[must_use]
    pub fn executable_slot(&self) -> u32 {
        self.slot_of(self.roles.len())
    }

    /// The first slot of the manifest role, if present.
    #[must_use]
    pub fn slot_for(&self, role: DescriptorRole) -> Option<u32> {
        self.roles
            .iter()
            .position(|candidate| *candidate == role)
            .map(|index| self.slot_of(index))
    }

    /// The comma-separated role encoding passed to the VMM as its only argument.
    #[must_use]
    pub fn encode(&self) -> String {
        self.roles
            .iter()
            .map(|role| role.token())
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Parses [`Self::encode`] output and re-validates it.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::UnknownToken`] or the structural errors of [`Self::new`].
    pub fn decode(encoded: &str) -> Result<Self, ManifestError> {
        let roles = encoded
            .split(',')
            .map(|token| DescriptorRole::from_token(token).ok_or(ManifestError::UnknownToken))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(roles)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_follow_manifest_order_after_the_standard_streams() {
        let manifest = DescriptorManifest::new(vec![
            DescriptorRole::Kvm,
            DescriptorRole::Control,
            DescriptorRole::Event,
        ])
        .expect("valid");
        assert_eq!(manifest.slot_of(0), 3);
        assert_eq!(manifest.slot_for(DescriptorRole::Control), Some(4));
        assert_eq!(manifest.slot_for(DescriptorRole::Event), Some(5));
        assert_eq!(manifest.executable_slot(), 6);
        assert_eq!(manifest.slot_for(DescriptorRole::Tap), None);
    }

    #[test]
    fn rejects_structural_mistakes() {
        assert_eq!(DescriptorManifest::new(vec![]), Err(ManifestError::Empty));
        assert_eq!(
            DescriptorManifest::new(vec![DescriptorRole::Kvm]),
            Err(ManifestError::MissingControl)
        );
        assert_eq!(
            DescriptorManifest::new(vec![DescriptorRole::Control, DescriptorRole::Control]),
            Err(ManifestError::DuplicateSingleton(DescriptorRole::Control))
        );
        let too_many = vec![DescriptorRole::Event; MAX_ROLES + 1];
        assert_eq!(
            DescriptorManifest::new(too_many),
            Err(ManifestError::TooLong { max: MAX_ROLES })
        );
    }

    #[test]
    fn encoding_round_trips_every_role() {
        let roles = vec![
            DescriptorRole::Kvm,
            DescriptorRole::Tap,
            DescriptorRole::RootDisk,
            DescriptorRole::OverlayHead,
            DescriptorRole::Artifact(ArtifactKind::Kernel),
            DescriptorRole::Artifact(ArtifactKind::Initramfs),
            DescriptorRole::Artifact(ArtifactKind::MemorySnapshot),
            DescriptorRole::Artifact(ArtifactKind::DeviceState),
            DescriptorRole::Control,
            DescriptorRole::Log,
            DescriptorRole::Event,
        ];
        let manifest = DescriptorManifest::new(roles).expect("valid");
        let encoded = manifest.encode();
        assert_eq!(
            encoded,
            "kvm,tap,root-disk,overlay-head,artifact.kernel,artifact.initramfs,\
             artifact.memory-snapshot,artifact.device-state,control,log,event"
        );
        assert_eq!(DescriptorManifest::decode(&encoded), Ok(manifest));
        assert_eq!(
            DescriptorManifest::decode("control,/dev/kvm"),
            Err(ManifestError::UnknownToken)
        );
    }
}
