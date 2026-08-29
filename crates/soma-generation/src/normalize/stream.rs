use std::{
    cell::Cell,
    io::{self, Read},
};

use sha2::{Digest as _, Sha256};
use soma::OciDigest;

use crate::{NormalizeError, NormalizeErrorKind, NormalizePhase, digest};

const TAR_BLOCK_BYTES: u64 = 512;

#[derive(Default)]
pub(super) struct StreamState {
    limit_exceeded: Cell<bool>,
    source_failed: Cell<bool>,
}

impl StreamState {
    pub(super) fn failure(&self) -> Option<NormalizeError> {
        if self.limit_exceeded.get() {
            Some(limit())
        } else if self.source_failed.get() {
            Some(NormalizeError::new(
                NormalizePhase::VerifyLayer,
                NormalizeErrorKind::Io,
            ))
        } else {
            None
        }
    }

    pub(super) fn error(&self) -> NormalizeError {
        self.failure().unwrap_or_else(integrity)
    }
}

pub(super) struct ExpandedStream<'a, R> {
    reader: R,
    hasher: Sha256,
    maximum: u64,
    total: u64,
    state: &'a StreamState,
}

impl<'a, R: Read> ExpandedStream<'a, R> {
    pub(super) fn new(reader: R, maximum: u64, state: &'a StreamState) -> Self {
        Self {
            reader,
            hasher: Sha256::new(),
            maximum,
            total: 0,
            state,
        }
    }

    pub(super) fn validate_tail(&mut self) -> Result<(), NormalizeError> {
        let mut buffer = [0_u8; 8 * 1024];
        let mut tail_bytes = 0_u64;
        loop {
            let count = self.read(&mut buffer).map_err(|_| self.error())?;
            if count == 0 {
                break;
            }
            if buffer[..count].iter().any(|byte| *byte != 0) {
                return Err(integrity());
            }
            tail_bytes = tail_bytes
                .checked_add(u64::try_from(count).map_err(|_| limit())?)
                .ok_or_else(limit)?;
        }
        if tail_bytes < TAR_BLOCK_BYTES || !tail_bytes.is_multiple_of(TAR_BLOCK_BYTES) {
            return Err(integrity());
        }
        Ok(())
    }

    pub(super) fn finish(self) -> (OciDigest, u64) {
        (
            digest::from_output(self.hasher.finalize().as_ref()),
            self.total,
        )
    }

    pub(super) fn error(&self) -> NormalizeError {
        self.state.error()
    }
}

impl<R: Read> Read for ExpandedStream<'_, R> {
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
                    Err(io::Error::other("expanded rootfs limit exceeded"))
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

const fn integrity() -> NormalizeError {
    NormalizeError::new(NormalizePhase::VerifyLayer, NormalizeErrorKind::Integrity)
}

const fn limit() -> NormalizeError {
    NormalizeError::new(
        NormalizePhase::VerifyLayer,
        NormalizeErrorKind::LimitExceeded,
    )
}
