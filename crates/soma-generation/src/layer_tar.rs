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

use crate::tar_preflight::{
    ExtensionPolicy, MAX_LOCAL_PAX_RECORD_BYTES, PreflightBudget, PreflightError,
};
use crate::{ImportError, ImportErrorKind, ImportPhase, digest};

mod budget;

pub(crate) use budget::{ValidationBudget, preflight_budget, validation_budget};

const BLOCK_BYTES: u64 = 512;
const MAX_PATH_BYTES: usize = 4_096;
#[cfg(test)]
use budget::{MAX_ENTRIES, MAX_PATH_METADATA_BYTES};

#[derive(Debug)]
pub(crate) struct ValidatedLayerTar {
    pub(crate) diff_id: OciDigest,
    pub(crate) expanded_size: u64,
    pub(crate) entry_count: u32,
}

pub(crate) fn preflight<R: Read>(
    reader: R,
    maximum_expanded_bytes: u64,
    budget: &mut PreflightBudget,
) -> Result<(), ImportError> {
    crate::tar_preflight::preflight(
        reader,
        maximum_expanded_bytes,
        ExtensionPolicy {
            long_record_ceiling: u64::try_from(MAX_PATH_BYTES).expect("maximum path bytes fit u64")
                + 1,
            pax_record_ceiling: MAX_LOCAL_PAX_RECORD_BYTES,
        },
        budget,
    )
    .map_err(map_preflight_error)
}

pub(crate) fn validate<R: Read>(
    reader: &mut R,
    maximum_expanded_bytes: u64,
    budget: &mut ValidationBudget,
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
                budget,
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
    entry_count: u32,
}

impl PathPolicy {
    fn observe(
        &mut self,
        path: &[u8],
        link: Option<&[u8]>,
        hard_link: bool,
        budget: &mut ValidationBudget,
    ) -> Result<(), ImportError> {
        let logical_path = normalize_path(path)?;
        if self.logical_paths.contains(&logical_path) {
            return Err(input_error());
        }
        let mut metadata_bytes = path.len();
        if let Some(link) = link {
            if hard_link {
                normalize_path(link)?;
            } else {
                validate_metadata_path(link)?;
            }
            metadata_bytes = metadata_bytes
                .checked_add(link.len())
                .ok_or_else(limit_error)?;
        }
        let next_entry_count = self.entry_count.checked_add(1).ok_or_else(limit_error)?;
        budget.observe(1, u64::try_from(metadata_bytes).map_err(|_| limit_error())?)?;
        self.entry_count = next_entry_count;
        self.logical_paths.insert(logical_path);
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

const fn map_preflight_error(error: PreflightError) -> ImportError {
    let kind = match error {
        PreflightError::Unsupported => ImportErrorKind::Unsupported,
        PreflightError::LimitExceeded => ImportErrorKind::LimitExceeded,
        PreflightError::Integrity => ImportErrorKind::Integrity,
        PreflightError::Io => ImportErrorKind::Io,
    };
    ImportError::new(ImportPhase::VerifyLayer, kind)
}

#[cfg(test)]
mod tests;
