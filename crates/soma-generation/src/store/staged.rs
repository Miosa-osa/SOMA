use std::{
    io::{Read, Seek as _, SeekFrom, Write as _},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{File, OpenOptions};
use soma::OciDigest;

use super::Store;
use crate::{ImportError, ImportErrorKind, ImportPhase, digest, oci::Descriptor};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StagedObject<'store> {
    store: &'store Store,
    name: String,
    file: Option<File>,
    descriptor: Descriptor,
}

impl Store {
    pub(crate) fn stage_descriptor<'store>(
        &'store self,
        source: &mut impl Read,
        descriptor: &Descriptor,
        maximum: u64,
        phase: ImportPhase,
    ) -> Result<StagedObject<'store>, ImportError> {
        if descriptor.size > maximum {
            return Err(ImportError::new(phase, ImportErrorKind::LimitExceeded));
        }
        let (name, file) = self.create_temporary()?;
        let mut staged = StagedObject {
            store: self,
            name,
            file: Some(file),
            descriptor: descriptor.clone(),
        };
        let (actual_digest, actual_size) =
            copy_hashed(source, staged.file_mut(phase)?, descriptor.size, phase)?;
        if actual_digest != descriptor.digest || actual_size != descriptor.size {
            return Err(ImportError::new(phase, ImportErrorKind::Integrity));
        }
        staged.prepare()?;
        Ok(staged)
    }

    fn create_temporary(&self) -> Result<(String, File), ImportError> {
        for _ in 0..32 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!("{}.{}", std::process::id(), sequence);
            let mut options = OpenOptions::new();
            options
                .read(true)
                .write(true)
                .create_new(true)
                .follow(FollowSymlinks::No);
            match self.temporary.open_with(&name, &options) {
                Ok(file) => return Ok((name, file)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(ImportError::new(ImportPhase::Publish, ImportErrorKind::Io));
                }
            }
        }
        Err(ImportError::new(
            ImportPhase::Publish,
            ImportErrorKind::StoreConflict,
        ))
    }
}

impl StagedObject<'_> {
    pub(crate) fn reader(&mut self) -> Result<&mut File, ImportError> {
        let file = self.file_mut(ImportPhase::VerifyLayer)?;
        file.seek(SeekFrom::Start(0))
            .map_err(|_| ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::Io))?;
        Ok(file)
    }

    pub(crate) fn publish(mut self) -> Result<(), ImportError> {
        let file = self
            .file
            .take()
            .ok_or_else(|| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?;
        self.store
            .publish_staged(&self.name, file, &self.descriptor)
    }

    fn prepare(&mut self) -> Result<(), ImportError> {
        let file = self.file_mut(ImportPhase::Publish)?;
        file.sync_all()
            .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?;
        #[cfg(not(windows))]
        {
            let mut permissions = file
                .metadata()
                .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?
                .permissions();
            permissions.set_readonly(true);
            file.set_permissions(permissions)
                .map_err(|_| ImportError::new(ImportPhase::Publish, ImportErrorKind::Io))?;
        }
        Ok(())
    }

    fn file_mut(&mut self, phase: ImportPhase) -> Result<&mut File, ImportError> {
        self.file
            .as_mut()
            .ok_or_else(|| ImportError::new(phase, ImportErrorKind::Io))
    }
}

impl Drop for StagedObject<'_> {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = self.store.temporary.remove_file(&self.name);
    }
}

pub(super) fn copy_hashed(
    source: &mut impl Read,
    target: &mut File,
    expected: u64,
    phase: ImportPhase,
) -> Result<(OciDigest, u64), ImportError> {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    while total < expected {
        let remaining = expected - total;
        let capacity = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
        let count = read_retry(source, &mut buffer[..capacity], phase)?;
        if count == 0 {
            return Err(ImportError::new(phase, ImportErrorKind::Integrity));
        }
        target
            .write_all(&buffer[..count])
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        hasher.update(&buffer[..count]);
        total += u64::try_from(count)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
    }
    if read_retry(source, &mut [0_u8], phase)? != 0 {
        return Err(ImportError::new(phase, ImportErrorKind::Integrity));
    }
    let output = hasher.finalize();
    Ok((digest::from_output(output.as_ref()), total))
}

fn read_retry(
    source: &mut impl Read,
    target: &mut [u8],
    phase: ImportPhase,
) -> Result<usize, ImportError> {
    loop {
        match source.read(target) {
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(ImportError::new(phase, ImportErrorKind::Io)),
        }
    }
}
