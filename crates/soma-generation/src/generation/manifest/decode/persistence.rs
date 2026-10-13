//! Decoding group 17, the durable data volume, beside the rest of the manifest.
//!
//! It lives here rather than in the decoder proper so that file stays inside the source budget;
//! the group is an ordinary part of the manifest and the reader is the same reader.

use super::super::{MAX_PERSISTENCE_PATHS, PersistenceBinding, is_volume_class};
use super::{Decoder, invalid};
use crate::generation::error::CompileError;

impl Decoder<'_> {
    pub(super) fn persistence(&mut self) -> Result<PersistenceBinding, CompileError> {
        self.tag(17)?;
        let enabled = match self.u8()? {
            0 => false,
            1 => true,
            _ => return Err(invalid()),
        };
        let size_class_gib = u32::from(self.u16()?);
        let count = usize::from(self.u8()?);
        if count > MAX_PERSISTENCE_PATHS {
            return Err(invalid());
        }
        let mut paths = Vec::with_capacity(count);
        for _ in 0..count {
            paths.push(self.short_string()?);
        }
        if !enabled && (!paths.is_empty() || size_class_gib != 0) {
            return Err(invalid());
        }
        if enabled && !is_volume_class(size_class_gib) {
            return Err(invalid());
        }
        Ok(PersistenceBinding {
            enabled,
            paths,
            size_class_gib,
        })
    }
}
