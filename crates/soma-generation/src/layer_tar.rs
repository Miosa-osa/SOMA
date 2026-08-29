//! Streaming structural validation for decompressed OCI layer tar streams.
//!
//! Entry paths and hard-link targets must remain beneath the future rootfs.
//! Symlink targets are only metadata-bounded here because safely resolving them belongs to the
//! rootfs applier, and valid Linux images can contain absolute or parent-relative symlinks.

use std::{
    cell::Cell,
    collections::BTreeSet,
    io::{self, Read},
};

use sha2::{Digest as _, Sha256};
use soma::OciDigest;

use crate::{ImportError, ImportErrorKind, ImportPhase, digest};

const BLOCK_BYTES: u64 = 512;
const MAX_ENTRIES: u32 = 1_000_000;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_PATH_METADATA_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct ValidatedLayerTar {
    pub(crate) diff_id: OciDigest,
    pub(crate) expanded_size: u64,
    pub(crate) entry_count: u32,
}

pub(crate) fn validate<R: Read>(
    reader: &mut R,
    maximum_expanded_bytes: u64,
) -> Result<ValidatedLayerTar, ImportError> {
    let state = StreamState::default();
    let mut stream = LayerStream::new(reader, maximum_expanded_bytes, &state);
    let mut paths = PathPolicy::default();
    {
        let mut archive = tar::Archive::new(&mut stream);
        let entries = archive.entries().map_err(|_| stream_error(&state))?;
        for entry in entries {
            let entry = entry.map_err(|_| stream_error(&state))?;
            paths.observe(
                entry.path_bytes().as_ref(),
                entry.link_name_bytes().as_deref(),
                entry.header().entry_type().is_hard_link(),
            )?;
        }
    }
    validate_tail(&mut stream, &state)?;
    let (diff_id, expanded_size) = stream.finish();
    Ok(ValidatedLayerTar {
        diff_id,
        expanded_size,
        entry_count: paths.entry_count,
    })
}

#[derive(Default)]
struct PathPolicy {
    logical_paths: BTreeSet<Vec<u8>>,
    path_metadata_bytes: u64,
    entry_count: u32,
}

impl PathPolicy {
    fn observe(
        &mut self,
        path: &[u8],
        link: Option<&[u8]>,
        hard_link: bool,
    ) -> Result<(), ImportError> {
        self.entry_count = self.entry_count.checked_add(1).ok_or_else(limit_error)?;
        if self.entry_count > MAX_ENTRIES {
            return Err(limit_error());
        }
        self.add_metadata(path.len())?;
        let logical_path = normalize_path(path)?;
        if !self.logical_paths.insert(logical_path) {
            return Err(input_error());
        }
        if let Some(link) = link {
            if hard_link {
                normalize_path(link)?;
            } else {
                validate_metadata_path(link)?;
            }
            self.add_metadata(link.len())?;
        }
        Ok(())
    }

    fn add_metadata(&mut self, length: usize) -> Result<(), ImportError> {
        self.path_metadata_bytes = self
            .path_metadata_bytes
            .checked_add(u64::try_from(length).map_err(|_| limit_error())?)
            .ok_or_else(limit_error)?;
        if self.path_metadata_bytes > MAX_PATH_METADATA_BYTES {
            return Err(limit_error());
        }
        Ok(())
    }
}

fn normalize_path(path: &[u8]) -> Result<Vec<u8>, ImportError> {
    validate_metadata_path(path)?;
    if path.is_empty() || path.starts_with(b"/") {
        return Err(input_error());
    }
    let mut normalized = Vec::with_capacity(path.len());
    for component in path.split(|byte| *byte == b'/') {
        match component {
            b"" | b"." => {}
            b".." => return Err(input_error()),
            component => {
                if !normalized.is_empty() {
                    normalized.push(b'/');
                }
                normalized.extend_from_slice(component);
            }
        }
    }
    if normalized.is_empty() {
        normalized.push(b'.');
    }
    Ok(normalized)
}

fn validate_metadata_path(path: &[u8]) -> Result<(), ImportError> {
    if path.len() > MAX_PATH_BYTES {
        return Err(limit_error());
    }
    if path.contains(&0) {
        return Err(input_error());
    }
    Ok(())
}

fn validate_tail<R: Read>(
    stream: &mut LayerStream<'_, R>,
    state: &StreamState,
) -> Result<(), ImportError> {
    let mut buffer = [0_u8; 8 * 1024];
    let mut tail_bytes = 0_u64;
    loop {
        let count = stream.read(&mut buffer).map_err(|_| stream_error(state))?;
        if count == 0 {
            break;
        }
        if buffer[..count].iter().any(|byte| *byte != 0) {
            return Err(integrity_error());
        }
        tail_bytes = tail_bytes
            .checked_add(u64::try_from(count).map_err(|_| limit_error())?)
            .ok_or_else(limit_error)?;
    }
    if tail_bytes < BLOCK_BYTES || !tail_bytes.is_multiple_of(BLOCK_BYTES) {
        return Err(integrity_error());
    }
    Ok(())
}

#[derive(Default)]
struct StreamState {
    limit_exceeded: Cell<bool>,
    source_failed: Cell<bool>,
}

struct LayerStream<'a, R> {
    reader: &'a mut R,
    hasher: Sha256,
    maximum: u64,
    total: u64,
    state: &'a StreamState,
}

impl<'a, R> LayerStream<'a, R> {
    fn new(reader: &'a mut R, maximum: u64, state: &'a StreamState) -> Self {
        Self {
            reader,
            hasher: Sha256::new(),
            maximum,
            total: 0,
            state,
        }
    }

    fn finish(self) -> (OciDigest, u64) {
        (
            digest::from_output(self.hasher.finalize().as_ref()),
            self.total,
        )
    }
}

impl<R: Read> Read for LayerStream<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let remaining = self.maximum.saturating_sub(self.total);
        if remaining == 0 {
            let mut probe = [0_u8; 1];
            return match self.reader.read(&mut probe) {
                Ok(0) => Ok(0),
                Ok(_) => {
                    self.state.limit_exceeded.set(true);
                    Err(io::Error::other("expanded layer limit exceeded"))
                }
                Err(error) => {
                    self.state.source_failed.set(true);
                    Err(error)
                }
            };
        }
        let allowed = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        match self.reader.read(&mut buffer[..allowed]) {
            Ok(count) => {
                self.hasher.update(&buffer[..count]);
                self.total += u64::try_from(count).expect("read count fits u64");
                Ok(count)
            }
            Err(error) => {
                self.state.source_failed.set(true);
                Err(error)
            }
        }
    }
}

fn stream_error(state: &StreamState) -> ImportError {
    if state.limit_exceeded.get() {
        limit_error()
    } else if state.source_failed.get() {
        ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::Io)
    } else {
        integrity_error()
    }
}

const fn input_error() -> ImportError {
    ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::InvalidInput)
}

const fn limit_error() -> ImportError {
    ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::LimitExceeded)
}

const fn integrity_error() -> ImportError {
    ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::Integrity)
}

#[cfg(test)]
mod tests;
