use crate::{Error, OutputChunk, TerminalReport, TerminalStatus};

pub(crate) struct OutputAccounting {
    allowance: u64,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl OutputAccounting {
    pub(crate) fn new(allowance: u64) -> Self {
        Self {
            allowance,
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    pub(crate) fn push_stdout(&mut self, chunk: &OutputChunk) -> Result<(), ()> {
        push(self.allowance, &mut self.stdout, self.stderr.len(), chunk)
    }

    pub(crate) fn push_stderr(&mut self, chunk: &OutputChunk) -> Result<(), ()> {
        push(self.allowance, &mut self.stderr, self.stdout.len(), chunk)
    }

    pub(crate) fn report(&self, status: TerminalStatus) -> Result<TerminalReport, ()> {
        if status == TerminalStatus::OutputLimit && self.total()? != self.allowance {
            return Err(());
        }
        TerminalReport::new(
            status,
            u32::try_from(self.stdout.len()).map_err(|_| ())?,
            u32::try_from(self.stderr.len()).map_err(|_| ())?,
        )
        .map_err(|_| ())
    }

    pub(crate) fn validate(&self, report: TerminalReport) -> Result<(), ()> {
        if usize::try_from(report.stdout_bytes()).map_err(|_| ())? != self.stdout.len()
            || usize::try_from(report.stderr_bytes()).map_err(|_| ())? != self.stderr.len()
            || (report.status() == TerminalStatus::OutputLimit && self.total()? != self.allowance)
        {
            return Err(());
        }
        Ok(())
    }

    pub(crate) fn into_output(self) -> (Box<[u8]>, Box<[u8]>) {
        (
            self.stdout.into_boxed_slice(),
            self.stderr.into_boxed_slice(),
        )
    }

    fn total(&self) -> Result<u64, ()> {
        u64::try_from(self.stdout.len())
            .ok()
            .and_then(|stdout| {
                u64::try_from(self.stderr.len())
                    .ok()
                    .and_then(|stderr| stdout.checked_add(stderr))
            })
            .ok_or(())
    }
}

pub(crate) fn output_chunk(bytes: Vec<u8>) -> Result<OutputChunk, Error> {
    OutputChunk::new(bytes)
}

fn push(
    allowance: u64,
    stream: &mut Vec<u8>,
    other_length: usize,
    chunk: &OutputChunk,
) -> Result<(), ()> {
    let total = stream
        .len()
        .checked_add(other_length)
        .and_then(|current| current.checked_add(chunk.as_bytes().len()))
        .and_then(|total| u64::try_from(total).ok())
        .ok_or(())?;
    if total > allowance {
        return Err(());
    }
    stream.extend_from_slice(chunk.as_bytes());
    Ok(())
}
