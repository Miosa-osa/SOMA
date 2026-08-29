use std::convert::TryInto;

use super::{
    Arm64BootError,
    command::{Arm64CommandOutcome, Arm64Terminal, PreparedCommand},
    protocol::{Frame, Kind},
};

const CHUNK_SIZE: usize = 4096;

pub(crate) struct ResponseCollector {
    request_id: u64,
    challenge: [u8; 32],
    output_limit: usize,
    sequence: u32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    finished: bool,
}

impl ResponseCollector {
    pub(crate) fn new(command: &PreparedCommand) -> Self {
        Self {
            request_id: command.request_id,
            challenge: command.challenge,
            output_limit: command.output_limit,
            sequence: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
            finished: false,
        }
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "the collector consumes guest payload bytes without cloning them"
    )]
    pub(crate) fn accept(
        &mut self,
        frame: Frame,
    ) -> Result<Option<Arm64CommandOutcome>, Arm64BootError> {
        if self.finished {
            return Err(Arm64BootError::message(
                "control response continued after terminal frame",
            ));
        }
        self.validate_identity(&frame)?;
        match frame.kind {
            Kind::Stdout => self.accept_chunk(true, frame.payload)?,
            Kind::Stderr => self.accept_chunk(false, frame.payload)?,
            Kind::Terminal => return self.accept_terminal(&frame.payload).map(Some),
            Kind::Hello | Kind::Request => {
                return Err(Arm64BootError::message(
                    "unexpected control frame after command request",
                ));
            }
        }
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| Arm64BootError::message("control response sequence overflow"))?;
        Ok(None)
    }

    fn validate_identity(&self, frame: &Frame) -> Result<(), Arm64BootError> {
        if frame.request_id != self.request_id
            || frame.challenge != self.challenge
            || frame.sequence != self.sequence
        {
            return Err(Arm64BootError::message(
                "control response identity or sequence mismatch",
            ));
        }
        Ok(())
    }

    fn accept_chunk(&mut self, stdout: bool, mut payload: Vec<u8>) -> Result<(), Arm64BootError> {
        if payload.is_empty() || payload.len() > CHUNK_SIZE {
            return Err(Arm64BootError::message(
                "control output chunk has invalid length",
            ));
        }
        let used = self
            .stdout
            .len()
            .checked_add(self.stderr.len())
            .and_then(|length| length.checked_add(payload.len()))
            .ok_or_else(|| Arm64BootError::message("combined output length overflow"))?;
        if used > self.output_limit {
            return Err(Arm64BootError::message(
                "guest exceeded the combined command output allowance",
            ));
        }
        if stdout {
            self.stdout.append(&mut payload);
        } else {
            self.stderr.append(&mut payload);
        }
        Ok(())
    }

    fn accept_terminal(&mut self, payload: &[u8]) -> Result<Arm64CommandOutcome, Arm64BootError> {
        if payload.len() != 16 || payload[1..4] != [0, 0, 0] {
            return Err(Arm64BootError::message("invalid terminal payload"));
        }
        let value = i32::from_be_bytes(payload[4..8].try_into().unwrap());
        let stdout_len = u32::from_be_bytes(payload[8..12].try_into().unwrap()) as usize;
        let stderr_len = u32::from_be_bytes(payload[12..16].try_into().unwrap()) as usize;
        if stdout_len != self.stdout.len() || stderr_len != self.stderr.len() {
            return Err(Arm64BootError::message(
                "terminal output counts do not match received chunks",
            ));
        }
        let terminal = match (payload[0], value) {
            (0, 0..=255) => Arm64Terminal::Exited(value),
            (1, 1..=64) => Arm64Terminal::Signaled(value),
            (2, 0) => Arm64Terminal::TimedOut,
            (3, 0) => Arm64Terminal::OutputLimit,
            (4, 1..=4095) => Arm64Terminal::ExecFailed(value),
            (5, 1..=4095) => Arm64Terminal::AgentFailed(value),
            _ => return Err(Arm64BootError::message("invalid terminal outcome value")),
        };
        if terminal == Arm64Terminal::OutputLimit
            && stdout_len.checked_add(stderr_len) != Some(self.output_limit)
        {
            return Err(Arm64BootError::message(
                "output-limit terminal did not retain the exact allowance",
            ));
        }
        self.finished = true;
        Ok(Arm64CommandOutcome {
            stdout: std::mem::take(&mut self.stdout),
            stderr: std::mem::take(&mut self.stderr),
            terminal,
        })
    }
}

pub(crate) fn validate_hello(frame: &Frame) -> Result<(), Arm64BootError> {
    if frame.kind != Kind::Hello
        || frame.request_id != 0
        || frame.sequence != 0
        || frame.challenge != [0; 32]
        || !frame.payload.is_empty()
    {
        return Err(Arm64BootError::message("invalid guest-agent hello frame"));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
