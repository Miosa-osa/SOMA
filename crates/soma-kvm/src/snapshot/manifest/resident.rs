//! Machine state of a running Instance, with a format distinct from Generation snapshots.

use std::{error::Error, fmt};

use super::{
    ManifestError, ManifestHeader, Reader, Section, SectionRole, Writer, read_sections,
    validate_sections, write_sections,
};

const MAGIC: [u8; 8] = *b"SOMACKP\0";
const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointManifestError {
    Manifest(ManifestError),
    BadMagic,
    UnsupportedVersion(u16),
    ZeroInstanceId,
    GenerationRepairPoint,
}

impl fmt::Display for CheckpointManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(formatter, "{error}"),
            Self::BadMagic => formatter.write_str("checkpoint magic is not SOMACKP"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported resident checkpoint version {version}"
                )
            }
            Self::ZeroInstanceId => formatter.write_str("checkpoint Instance id cannot be zero"),
            Self::GenerationRepairPoint => {
                formatter.write_str("resident checkpoints cannot carry a Generation repair point")
            }
        }
    }
}

impl Error for CheckpointManifestError {}

impl From<ManifestError> for CheckpointManifestError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<crate::snapshot::WireError> for CheckpointManifestError {
    fn from(error: crate::snapshot::WireError) -> Self {
        Self::Manifest(error.into())
    }
}

/// The architecture state of one resident Instance.
///
/// This value describes bytes. The broker must authorize the expected Instance and authenticate
/// the checkpoint and its artifacts before allowing a restore.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointManifest {
    instance_id: [u8; 16],
    header: ManifestHeader,
    sections: Vec<Section>,
}

impl CheckpointManifest {
    pub(crate) fn state(&self) -> super::StateView<'_> {
        super::StateView {
            header: &self.header,
            sections: &self.sections,
        }
    }

    /// Constructs a typed resident checkpoint manifest from canonical sections.
    ///
    /// # Errors
    /// Returns an error for a zero Instance id, a Generation marker, or invalid section structure.
    pub fn new(
        instance_id: [u8; 16],
        header: ManifestHeader,
        sections: Vec<Section>,
    ) -> Result<Self, CheckpointManifestError> {
        if instance_id == [0; 16] {
            return Err(CheckpointManifestError::ZeroInstanceId);
        }
        if sections
            .iter()
            .any(|section| section.role() == SectionRole::RepairPointMarker)
        {
            return Err(CheckpointManifestError::GenerationRepairPoint);
        }
        validate_sections(&sections, false)?;
        Ok(Self {
            instance_id,
            header,
            sections,
        })
    }

    #[must_use]
    pub const fn instance_id(&self) -> &[u8; 16] {
        &self.instance_id
    }

    #[must_use]
    pub const fn header(&self) -> &ManifestHeader {
        &self.header
    }

    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(4096);
        writer.put_bytes(&MAGIC);
        writer.put_u16(VERSION);
        writer.put_bytes(&self.instance_id);
        self.header.encode(&mut writer);
        write_sections(&mut writer, &self.sections);
        writer.finish()
    }

    /// Decodes hostile bytes using the bounded architecture section codec.
    ///
    /// # Errors
    /// Returns an error for a wrong format, bad identity, invalid sections, or trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, CheckpointManifestError> {
        let mut reader = Reader::new(bytes);
        if reader.array::<8>()? != MAGIC {
            return Err(CheckpointManifestError::BadMagic);
        }
        let version = reader.u16()?;
        if version != VERSION {
            return Err(CheckpointManifestError::UnsupportedVersion(version));
        }
        let instance_id = reader.array::<16>()?;
        let header = ManifestHeader::decode(&mut reader)?;
        let sections = read_sections(&mut reader)?;
        reader.finish()?;
        Self::new(instance_id, header, sections)
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckpointManifest, CheckpointManifestError};
    use crate::snapshot::{
        manifest::{Manifest, ManifestError, tests::sample_manifest},
        section::SectionRole,
    };

    #[test]
    fn a_checkpoint_is_bound_to_its_instance_and_cannot_open_as_a_generation() {
        let generation = sample_manifest();
        let sections = generation
            .sections()
            .iter()
            .filter(|section| section.role() != SectionRole::RepairPointMarker)
            .cloned()
            .collect();
        let checkpoint =
            CheckpointManifest::new([7; 16], generation.header().clone(), sections).unwrap();
        let mut encoded = checkpoint.encode();
        let decoded = CheckpointManifest::decode(&encoded).unwrap();
        assert_eq!(decoded, checkpoint);
        assert_eq!(decoded.instance_id(), &[7; 16]);
        assert_eq!(Manifest::decode(&encoded), Err(ManifestError::BadMagic));
        assert_eq!(
            CheckpointManifest::decode(&generation.encode()),
            Err(CheckpointManifestError::BadMagic)
        );
        encoded.push(0);
        assert!(CheckpointManifest::decode(&encoded).is_err());
    }

    #[test]
    fn a_checkpoint_rejects_generation_markers_missing_state_and_zero_identity() {
        let generation = sample_manifest();
        let header = generation.header();
        let all = generation.sections();
        assert_eq!(
            CheckpointManifest::new([7; 16], header.clone(), all.to_vec()),
            Err(CheckpointManifestError::GenerationRepairPoint)
        );

        let incomplete: Vec<_> = all
            .iter()
            .filter(|section| {
                !matches!(
                    section.role(),
                    SectionRole::RepairPointMarker | SectionRole::Vcpu0
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            CheckpointManifest::new([0; 16], header.clone(), incomplete.clone()),
            Err(CheckpointManifestError::ZeroInstanceId)
        );
        assert_eq!(
            CheckpointManifest::new([7; 16], header.clone(), incomplete),
            Err(CheckpointManifestError::Manifest(
                ManifestError::MissingRequiredSection(SectionRole::Vcpu0)
            ))
        );
    }
}
