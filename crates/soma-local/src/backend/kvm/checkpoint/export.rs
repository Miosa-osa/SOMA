use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::Path,
};

use sha2::{Digest as _, Sha256};
use soma_kvm::snapshot::Digest;

use crate::{CheckpointFailure, CheckpointReceipt};

use super::{MEMORY, OVERLAY, STATE, VOLUME, configured_root, open_private, read_published};

/// One artifact from the fixed checkpoint format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub enum CheckpointArtifact {
    State,
    Memory,
    Overlay,
    Workspace,
}

impl CheckpointArtifact {
    /// The artifact's filename in a published checkpoint.
    #[must_use]
    pub const fn filename(self) -> &'static str {
        match self {
            Self::State => STATE,
            Self::Memory => MEMORY,
            Self::Overlay => OVERLAY,
            Self::Workspace => VOLUME,
        }
    }
}

/// Read handles pinned to one verified capture.
pub struct CheckpointExport {
    artifacts: [Option<Artifact>; 4],
}

impl CheckpointExport {
    pub(super) fn verify_and_sync(&mut self) -> Result<(), CheckpointFailure> {
        for artifact in self.artifacts.iter_mut().flatten() {
            artifact.write_to(&mut std::io::sink())?;
            artifact
                .file
                .sync_all()
                .map_err(|_| CheckpointFailure::Storage)?;
        }
        Ok(())
    }

    /// Streams and verifies an artifact. An absent optional artifact writes nothing.
    ///
    /// # Errors
    ///
    /// Returns an error on I/O or digest mismatch. The destination must remain unpublished
    /// until the complete export is verified, since a failed call can leave partial output.
    pub fn write_artifact(
        &mut self,
        artifact: CheckpointArtifact,
        destination: &mut impl Write,
    ) -> Result<Option<u64>, CheckpointFailure> {
        self.artifacts[artifact as usize]
            .as_mut()
            .map(|input| input.write_to(destination))
            .transpose()
    }
}

struct Artifact {
    file: File,
    length: u64,
    digest: String,
}

impl Artifact {
    fn write_to(&mut self, destination: &mut impl Write) -> Result<u64, CheckpointFailure> {
        let storage = |_| CheckpointFailure::Storage;
        self.file.rewind().map_err(storage)?;
        let mut remaining = self.length;
        let mut hash = Sha256::new();
        let mut buffer = vec![0_u8; 128 * 1024];
        while remaining != 0 {
            let length = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| CheckpointFailure::Storage)?;
            let chunk = &mut buffer[..length];
            self.file.read_exact(chunk).map_err(storage)?;
            hash.update(&*chunk);
            destination.write_all(chunk).map_err(storage)?;
            remaining -= length as u64;
        }
        let digest = Digest::from_bytes(hash.finalize().into());
        if self.file.read(&mut buffer[..1]).map_err(storage)? != 0
            || format!("sha256:{digest}") != self.digest
        {
            return Err(CheckpointFailure::Storage);
        }
        Ok(self.length)
    }
}

pub(crate) fn open_export(
    expected: &CheckpointReceipt,
) -> Result<CheckpointExport, CheckpointFailure> {
    let root = configured_root()?;
    super::require_encrypted(&root)?;
    open_at(&super::published_path(&root, expected), expected)
}

pub(super) fn open_at(
    path: &Path,
    expected: &CheckpointReceipt,
) -> Result<CheckpointExport, CheckpointFailure> {
    let published = read_published(
        path,
        &expected.instance_id,
        &expected.generation_id,
        &expected.checkpoint_id,
    )?
    .ok_or(CheckpointFailure::NotFound)?;
    if published.receipt != *expected {
        return Err(CheckpointFailure::CaptureRefused);
    }

    let open = |kind: CheckpointArtifact, digest: &str, expected_length: Option<u64>| {
        let file = open_private(&path.join(kind.filename()))?;
        let length = file
            .metadata()
            .map_err(|_| CheckpointFailure::Storage)?
            .len();
        if expected_length.is_some_and(|expected| expected != length) {
            return Err(CheckpointFailure::Storage);
        }
        Ok(Artifact {
            file,
            length,
            digest: digest.to_owned(),
        })
    };
    Ok(CheckpointExport {
        artifacts: [
            Some(open(
                CheckpointArtifact::State,
                &expected.state_digest,
                None,
            )?),
            Some(open(
                CheckpointArtifact::Memory,
                &expected.memory_digest,
                Some(expected.memory_bytes),
            )?),
            expected
                .overlay_digest
                .as_deref()
                .map(|digest| {
                    open(
                        CheckpointArtifact::Overlay,
                        digest,
                        Some(expected.overlay_bytes),
                    )
                })
                .transpose()?,
            expected
                .volume
                .as_ref()
                .map(|volume| {
                    open(
                        CheckpointArtifact::Workspace,
                        &volume.digest,
                        Some(volume.bytes),
                    )
                })
                .transpose()?,
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_pinned_artifact_can_be_verified_again_after_unlink() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("artifact");
        let bytes = vec![42; 512 * 1024];
        fs::write(&path, &bytes).unwrap();
        let mut artifact = Artifact {
            file: File::open(&path).unwrap(),
            length: bytes.len() as u64,
            digest: format!("sha256:{}", Digest::of(&bytes)),
        };
        fs::remove_file(path).unwrap();
        for _ in 0..2 {
            let mut output = Vec::new();
            assert_eq!(artifact.write_to(&mut output), Ok(bytes.len() as u64));
            assert_eq!(output, bytes);
        }
    }

    #[test]
    fn changed_or_resized_artifacts_never_return_a_successful_transfer() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"corrupt").unwrap();
        for length in [6, 7, 8] {
            let mut artifact = Artifact {
                file: file.try_clone().unwrap(),
                length,
                digest: format!("sha256:{}", Digest::of(b"correct")),
            };
            assert_eq!(
                artifact.write_to(&mut std::io::sink()),
                Err(CheckpointFailure::Storage)
            );
        }
    }
}
