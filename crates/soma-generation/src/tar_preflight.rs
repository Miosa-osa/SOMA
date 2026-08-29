use std::{
    cell::Cell,
    io::{self, Read},
};

#[cfg(test)]
mod tests;

pub(crate) const MAX_LOCAL_PAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct ExtensionPolicy {
    pub(crate) long_record_ceiling: u64,
    pub(crate) pax_record_ceiling: u64,
}

pub(crate) struct PreflightBudget {
    raw_headers: u32,
    extension_bytes: u64,
    raw_header_ceiling: u32,
    extension_byte_ceiling: u64,
}

impl PreflightBudget {
    pub(crate) const fn new(raw_header_ceiling: u32, extension_byte_ceiling: u64) -> Self {
        Self {
            raw_headers: 0,
            extension_bytes: 0,
            raw_header_ceiling,
            extension_byte_ceiling,
        }
    }

    fn observe_header(&mut self) -> Result<(), PreflightError> {
        self.raw_headers = self
            .raw_headers
            .checked_add(1)
            .ok_or(PreflightError::LimitExceeded)?;
        if self.raw_headers > self.raw_header_ceiling {
            return Err(PreflightError::LimitExceeded);
        }
        Ok(())
    }

    fn observe_extension(&mut self, bytes: u64) -> Result<(), PreflightError> {
        self.extension_bytes = self
            .extension_bytes
            .checked_add(bytes)
            .ok_or(PreflightError::LimitExceeded)?;
        if self.extension_bytes > self.extension_byte_ceiling {
            return Err(PreflightError::LimitExceeded);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreflightError {
    Unsupported,
    LimitExceeded,
    Integrity,
    Io,
}

pub(crate) fn preflight(
    reader: impl Read,
    maximum: u64,
    policy: ExtensionPolicy,
    budget: &mut PreflightBudget,
) -> Result<(), PreflightError> {
    let state = ReadState::default();
    let mut bounded = BoundedReader::new(reader, maximum, &state);
    let mut pending = PendingExtensions::default();
    {
        let mut archive = tar::Archive::new(&mut bounded);
        let entries = archive.entries().map_err(|_| state.error())?.raw(true);
        for entry in entries {
            let entry = entry.map_err(|_| state.error())?;
            budget.observe_header()?;
            inspect_extension(&entry, policy, budget, &mut pending)?;
        }
    }
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let count = bounded.read(&mut buffer).map_err(|_| state.error())?;
        if count == 0 {
            break;
        }
    }
    Ok(())
}

fn inspect_extension<R: Read>(
    entry: &tar::Entry<'_, R>,
    policy: ExtensionPolicy,
    budget: &mut PreflightBudget,
    pending: &mut PendingExtensions,
) -> Result<(), PreflightError> {
    let kind = entry.header().entry_type();
    if kind.is_gnu_sparse() {
        return Err(PreflightError::Unsupported);
    }
    if kind.is_pax_global_extensions() {
        return Err(PreflightError::Unsupported);
    }
    if kind.is_pax_local_extensions() {
        if pending.gnu_naming {
            return Err(PreflightError::Unsupported);
        }
        if entry.size() > policy.pax_record_ceiling {
            return Err(PreflightError::LimitExceeded);
        }
        budget.observe_extension(entry.size())?;
        pending.local_pax = true;
    } else if kind.is_gnu_longname() || kind.is_gnu_longlink() {
        if pending.local_pax {
            return Err(PreflightError::Unsupported);
        }
        if entry.size() > policy.long_record_ceiling {
            return Err(PreflightError::LimitExceeded);
        }
        budget.observe_extension(entry.size())?;
        pending.gnu_naming = true;
    } else {
        *pending = PendingExtensions::default();
    }
    Ok(())
}

#[derive(Default)]
struct PendingExtensions {
    local_pax: bool,
    gnu_naming: bool,
}

#[derive(Default)]
struct ReadState {
    limit_exceeded: Cell<bool>,
    source_failed: Cell<bool>,
}

impl ReadState {
    fn error(&self) -> PreflightError {
        if self.limit_exceeded.get() {
            PreflightError::LimitExceeded
        } else if self.source_failed.get() {
            PreflightError::Io
        } else {
            PreflightError::Integrity
        }
    }
}

struct BoundedReader<'state, R> {
    reader: R,
    maximum: u64,
    total: u64,
    state: &'state ReadState,
}

impl<'state, R> BoundedReader<'state, R> {
    const fn new(reader: R, maximum: u64, state: &'state ReadState) -> Self {
        Self {
            reader,
            maximum,
            total: 0,
            state,
        }
    }
}

impl<R: Read> Read for BoundedReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let remaining = self.maximum.saturating_sub(self.total);
        if remaining == 0 {
            let mut probe = [0_u8; 1];
            return match self.reader.read(&mut probe) {
                Ok(0) => Ok(0),
                Ok(_) => {
                    self.state.limit_exceeded.set(true);
                    Err(io::Error::other("tar preflight limit exceeded"))
                }
                Err(error) => {
                    self.state.source_failed.set(true);
                    Err(error)
                }
            };
        }
        let allowed = usize::try_from(remaining)
            .unwrap_or(usize::MAX)
            .min(output.len());
        match self.reader.read(&mut output[..allowed]) {
            Ok(count) => {
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
