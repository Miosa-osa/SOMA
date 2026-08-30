//! Installation-time verification of the three immutable snapshot artifacts.

use std::{error::Error, fmt};

use super::{
    Digest,
    device_state::{DeviceSpecific, DeviceState, DeviceStateError},
    manifest::{CandidateId, Manifest, ManifestError},
    memory::MemoryError,
    section::SectionRole,
};

/// Digest and exact byte length measured through one retained artifact handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactEvidence {
    pub digest: Digest,
    pub size: u64,
}

/// The exact Candidate and artifact descriptors a ready Generation will bind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureExpectation {
    pub candidate_id: CandidateId,
    pub memory: ArtifactEvidence,
    pub overlay: ArtifactEvidence,
    pub state: ArtifactEvidence,
}

/// Facts independently verified before a captured Candidate may be certified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedCapture {
    pub candidate_id: CandidateId,
    pub memory: ArtifactEvidence,
    pub overlay: ArtifactEvidence,
    pub state: ArtifactEvidence,
}

/// A fail-closed snapshot installation rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InspectionError {
    Manifest(ManifestError),
    Memory(MemoryError),
    CandidateMismatch,
    MemoryBinding,
    OverlayBinding,
    StateBinding,
    MissingOverlayState,
    OverlayState(DeviceStateError),
    OverlayDigest,
    OverlaySize,
}

impl fmt::Display for InspectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(formatter, "snapshot manifest: {error}"),
            Self::Memory(error) => write!(formatter, "snapshot memory: {error}"),
            Self::CandidateMismatch => formatter.write_str("snapshot names a different Candidate"),
            Self::MemoryBinding => {
                formatter.write_str("snapshot memory differs from its Generation binding")
            }
            Self::OverlayBinding => {
                formatter.write_str("snapshot overlay differs from its Generation binding")
            }
            Self::StateBinding => {
                formatter.write_str("snapshot state differs from its Generation binding")
            }
            Self::MissingOverlayState => formatter.write_str("snapshot has no overlay state"),
            Self::OverlayState(error) => write!(formatter, "snapshot overlay state: {error}"),
            Self::OverlayDigest => {
                formatter.write_str("snapshot overlay digest does not match its device state")
            }
            Self::OverlaySize => {
                formatter.write_str("snapshot overlay size does not match its device capacity")
            }
        }
    }
}

impl Error for InspectionError {}

/// Verifies the canonical state and the exact memory and overlay objects it names.
///
/// The caller computes artifact evidence through already opened immutable handles.
/// This function performs no KVM ioctl and is deliberately outside the warm restore path.
///
/// # Errors
///
/// Returns the first malformed, mismatched, or missing identity or artifact fact.
pub fn inspect_capture(
    state_bytes: &[u8],
    memory: ArtifactEvidence,
    overlay: ArtifactEvidence,
    expected: CaptureExpectation,
) -> Result<VerifiedCapture, InspectionError> {
    let manifest = Manifest::decode(state_bytes).map_err(InspectionError::Manifest)?;
    if manifest.header().candidate_id != expected.candidate_id {
        return Err(InspectionError::CandidateMismatch);
    }
    if memory != expected.memory {
        return Err(InspectionError::MemoryBinding);
    }
    if overlay != expected.overlay {
        return Err(InspectionError::OverlayBinding);
    }
    let state = ArtifactEvidence {
        digest: Digest::of(state_bytes),
        size: u64::try_from(state_bytes.len()).unwrap_or(u64::MAX),
    };
    if state != expected.state {
        return Err(InspectionError::StateBinding);
    }
    manifest
        .header()
        .memory
        .verify_generation(memory.digest, memory.size)
        .map_err(InspectionError::Memory)?;
    let section = manifest
        .section(SectionRole::Device1)
        .ok_or(InspectionError::MissingOverlayState)?;
    let device = DeviceState::decode_for_slot(1, section.payload())
        .map_err(InspectionError::OverlayState)?;
    let DeviceSpecific::Block(block) = device.specific() else {
        return Err(InspectionError::OverlayState(
            DeviceStateError::SpecificMismatch(device.kind()),
        ));
    };
    if block.image_digest != overlay.digest {
        return Err(InspectionError::OverlayDigest);
    }
    if block.capacity_sectors.checked_mul(512) != Some(overlay.size) {
        return Err(InspectionError::OverlaySize);
    }
    Ok(VerifiedCapture {
        candidate_id: manifest.header().candidate_id,
        memory,
        overlay,
        state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        device_state::{DeviceKind, tests::sample as sample_device},
        manifest::tests::sample_manifest,
    };

    fn fixture() -> (
        Vec<u8>,
        ArtifactEvidence,
        ArtifactEvidence,
        CaptureExpectation,
    ) {
        let manifest = sample_manifest();
        let memory = ArtifactEvidence {
            digest: manifest.header().memory.digest(),
            size: manifest.header().memory.size(),
        };
        let DeviceSpecific::Block(block) = sample_device(DeviceKind::OverlayBlock).specific()
        else {
            unreachable!();
        };
        let overlay = ArtifactEvidence {
            digest: block.image_digest,
            size: block.capacity_sectors * 512,
        };
        let state = manifest.encode();
        let expected = CaptureExpectation {
            candidate_id: manifest.header().candidate_id,
            memory,
            overlay,
            state: ArtifactEvidence {
                digest: Digest::of(&state),
                size: u64::try_from(state.len()).unwrap(),
            },
        };
        (state, memory, overlay, expected)
    }

    #[test]
    fn verifies_the_exact_candidate_and_all_three_artifacts() {
        let (state, memory, overlay, expected) = fixture();
        let verified = inspect_capture(&state, memory, overlay, expected).unwrap();
        assert_eq!(verified.memory, memory);
        assert_eq!(verified.overlay, overlay);
        assert_eq!(verified.state.digest, Digest::of(&state));
        assert_eq!(verified.state.size, u64::try_from(state.len()).unwrap());
    }

    #[test]
    fn rejects_a_different_candidate_before_artifact_admission() {
        let (state, memory, overlay, mut expected) = fixture();
        let mut candidate = *expected.candidate_id.as_bytes();
        candidate[0] ^= 1;
        expected.candidate_id = CandidateId::new(candidate).unwrap();
        assert_eq!(
            inspect_capture(&state, memory, overlay, expected),
            Err(InspectionError::CandidateMismatch)
        );
    }

    #[test]
    fn rejects_memory_and_overlay_substitution() {
        let (state, memory, overlay, expected) = fixture();
        assert!(matches!(
            inspect_capture(
                &state,
                ArtifactEvidence {
                    size: memory.size - 1,
                    ..memory
                },
                overlay,
                expected
            ),
            Err(InspectionError::MemoryBinding)
        ));
        assert_eq!(
            inspect_capture(
                &state,
                memory,
                ArtifactEvidence {
                    digest: Digest::of(b"substitute"),
                    ..overlay
                },
                expected
            ),
            Err(InspectionError::OverlayBinding)
        );
        assert_eq!(
            inspect_capture(
                &state,
                memory,
                ArtifactEvidence {
                    size: overlay.size - 512,
                    ..overlay
                },
                expected
            ),
            Err(InspectionError::OverlayBinding)
        );
    }

    #[test]
    fn rejects_a_self_consistent_capture_substituted_for_the_bound_state() {
        let (mut state, memory, overlay, expected) = fixture();
        let last = state.len() - 1;
        state[last] ^= 1;
        assert!(inspect_capture(&state, memory, overlay, expected).is_err());
    }
}
