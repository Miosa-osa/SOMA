use crate::{ImportError, ImportErrorKind, ImportPhase, tar_preflight::PreflightBudget};

#[cfg(test)]
mod tests;

pub(super) const MAX_ENTRIES: u32 = 1_000_000;
pub(super) const MAX_PATH_METADATA_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) const fn preflight_budget() -> PreflightBudget {
    PreflightBudget::new(MAX_ENTRIES, MAX_PATH_METADATA_BYTES)
}

pub(crate) const fn validation_budget() -> ValidationBudget {
    ValidationBudget::new(MAX_ENTRIES, MAX_PATH_METADATA_BYTES)
}

pub(crate) struct ValidationBudget {
    entries: u32,
    path_metadata_bytes: u64,
    entry_ceiling: u32,
    path_metadata_ceiling: u64,
}

impl ValidationBudget {
    pub(crate) const fn new(entry_ceiling: u32, path_metadata_ceiling: u64) -> Self {
        Self {
            entries: 0,
            path_metadata_bytes: 0,
            entry_ceiling,
            path_metadata_ceiling,
        }
    }

    pub(crate) fn observe(
        &mut self,
        entries: u32,
        path_metadata_bytes: u64,
    ) -> Result<(), ImportError> {
        let next_entries = self.entries.checked_add(entries).ok_or_else(limit_error)?;
        let next_path_metadata_bytes = self
            .path_metadata_bytes
            .checked_add(path_metadata_bytes)
            .ok_or_else(limit_error)?;
        if next_entries > self.entry_ceiling
            || next_path_metadata_bytes > self.path_metadata_ceiling
        {
            return Err(limit_error());
        }
        self.entries = next_entries;
        self.path_metadata_bytes = next_path_metadata_bytes;
        Ok(())
    }
}

const fn limit_error() -> ImportError {
    ImportError::new(ImportPhase::VerifyLayer, ImportErrorKind::LimitExceeded)
}
