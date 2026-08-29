use std::{fmt, fs::File, io::Read, path::Path, time::Duration};

use super::{
    Arm64BootError,
    protocol::{self, Frame, Kind},
};

const MAX_ARGS: usize = 64;
const MAX_FIELD: usize = 4096;
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(crate) struct Arm64Fixtures<'a> {
    pub(crate) kernel: &'a Path,
    pub(crate) initramfs: &'a Path,
}

#[derive(Clone, Copy)]
pub(crate) struct Arm64Command<'a> {
    pub(crate) program: &'a str,
    pub(crate) args: &'a [&'a str],
    pub(crate) timeout: Duration,
    pub(crate) output_limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Arm64Terminal {
    Exited(i32),
    Signaled(i32),
    TimedOut,
    OutputLimit,
    ExecFailed(i32),
    AgentFailed(i32),
}

pub(crate) struct Arm64CommandOutcome {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) terminal: Arm64Terminal,
}

impl fmt::Debug for Arm64CommandOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Arm64CommandOutcome")
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .field("terminal", &self.terminal)
            .finish()
    }
}

pub(crate) struct PreparedCommand {
    pub(crate) request: Vec<u8>,
    pub(crate) request_id: u64,
    pub(crate) challenge: [u8; 32],
    pub(crate) timeout: Duration,
    pub(crate) output_limit: usize,
}

pub(crate) fn prepare(command: Arm64Command<'_>) -> Result<PreparedCommand, Arm64BootError> {
    validate(command)?;
    let (request_id, challenge) = random_identity()?;
    prepare_with_identity(command, request_id, challenge)
}

fn prepare_with_identity(
    command: Arm64Command<'_>,
    request_id: u64,
    challenge: [u8; 32],
) -> Result<PreparedCommand, Arm64BootError> {
    let (timeout_ms, payload_len) = validate(command)?;
    if request_id == 0 || challenge == [0; 32] {
        return Err(Arm64BootError::message(
            "command identity uses a reserved zero value",
        ));
    }
    let mut payload = Vec::with_capacity(payload_len);
    payload.extend_from_slice(&timeout_ms.to_be_bytes());
    payload.extend_from_slice(
        &u32::try_from(command.output_limit)
            .map_err(|error| Arm64BootError::at("encode command output limit", error))?
            .to_be_bytes(),
    );
    append_field(&mut payload, command.program)?;
    payload.extend_from_slice(
        &u16::try_from(command.args.len())
            .map_err(|error| Arm64BootError::at("encode command argument count", error))?
            .to_be_bytes(),
    );
    for argument in command.args {
        append_field(&mut payload, argument)?;
    }
    debug_assert_eq!(payload.len(), payload_len);
    let request = protocol::encode(&Frame {
        kind: Kind::Request,
        request_id,
        sequence: 0,
        challenge,
        payload,
    })
    .map_err(|error| Arm64BootError::at("encode command request", error))?;
    Ok(PreparedCommand {
        request,
        request_id,
        challenge,
        timeout: Duration::from_millis(u64::from(timeout_ms)),
        output_limit: command.output_limit,
    })
}

fn validate(command: Arm64Command<'_>) -> Result<(u32, usize), Arm64BootError> {
    if command.program.is_empty() || !command.program.starts_with('/') {
        return Err(Arm64BootError::message(
            "command program must be a nonempty absolute path",
        ));
    }
    validate_field("program", command.program)?;
    if command.args.len() > MAX_ARGS {
        return Err(Arm64BootError::message("command has too many arguments"));
    }
    for argument in command.args {
        validate_field("argument", argument)?;
    }
    let timeout_ms = command.timeout.as_millis();
    if timeout_ms == 0
        || command.timeout > MAX_TIMEOUT
        || !command.timeout.subsec_nanos().is_multiple_of(1_000_000)
    {
        return Err(Arm64BootError::message(
            "command timeout must be whole milliseconds from one through 30000",
        ));
    }
    if command.output_limit == 0 || command.output_limit > MAX_OUTPUT {
        return Err(Arm64BootError::message(
            "combined command output limit must be from one byte through 64 KiB",
        ));
    }
    let mut payload_len = 12_usize
        .checked_add(command.program.len())
        .ok_or_else(|| Arm64BootError::message("encoded command length overflow"))?;
    for argument in command.args {
        payload_len = payload_len
            .checked_add(2)
            .and_then(|length| length.checked_add(argument.len()))
            .ok_or_else(|| Arm64BootError::message("encoded command length overflow"))?;
    }
    if payload_len > protocol::MAX_PAYLOAD {
        return Err(Arm64BootError::message(
            "encoded command exceeds protocol payload limit",
        ));
    }
    let timeout_ms = u32::try_from(timeout_ms)
        .map_err(|error| Arm64BootError::at("encode command timeout", error))?;
    Ok((timeout_ms, payload_len))
}

fn validate_field(name: &str, value: &str) -> Result<(), Arm64BootError> {
    if value.len() > MAX_FIELD {
        return Err(Arm64BootError::message(format!(
            "command {name} exceeds 4096 bytes"
        )));
    }
    if value.as_bytes().contains(&0) {
        return Err(Arm64BootError::message(format!(
            "command {name} contains NUL"
        )));
    }
    Ok(())
}

fn append_field(payload: &mut Vec<u8>, value: &str) -> Result<(), Arm64BootError> {
    let length = u16::try_from(value.len())
        .map_err(|error| Arm64BootError::at("encode command field length", error))?;
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(value.as_bytes());
    Ok(())
}

fn random_identity() -> Result<(u64, [u8; 32]), Arm64BootError> {
    let mut source = File::open("/dev/urandom")
        .map_err(|error| Arm64BootError::at("open operating-system random source", error))?;
    for _ in 0..4 {
        let mut random = [0_u8; 40];
        source.read_exact(&mut random).map_err(|error| {
            Arm64BootError::at("read launch challenge from operating system", error)
        })?;
        let mut id = [0_u8; 8];
        id.copy_from_slice(&random[..8]);
        let request_id = u64::from_be_bytes(id);
        let mut challenge = [0_u8; 32];
        challenge.copy_from_slice(&random[8..]);
        if request_id != 0 && challenge != [0; 32] {
            return Ok((request_id, challenge));
        }
    }
    Err(Arm64BootError::message(
        "operating-system random source returned reserved zero identity",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command<'a>(program: &'a str, args: &'a [&'a str]) -> Arm64Command<'a> {
        Arm64Command {
            program,
            args,
            timeout: Duration::from_secs(1),
            output_limit: 1024,
        }
    }

    #[test]
    fn rejects_shell_like_or_unbounded_inputs_before_encoding() {
        assert!(prepare_with_identity(command("probe", &[]), 1, [2; 32]).is_err());
        assert!(prepare_with_identity(command("/probe\0bad", &[]), 1, [2; 32]).is_err());
        let mut invalid = command("/probe", &[]);
        invalid.output_limit = MAX_OUTPUT + 1;
        assert!(prepare_with_identity(invalid, 1, [2; 32]).is_err());
        assert!(prepare_with_identity(command("/probe", &[]), 1, [0; 32]).is_err());
    }

    #[test]
    fn request_carries_exact_identity_and_arguments() {
        let prepared = prepare_with_identity(command("/probe", &["ok", ""]), 7, [9; 32]).unwrap();
        let mut decoder = protocol::Decoder::new();
        let frame = prepared
            .request
            .iter()
            .find_map(|byte| decoder.push(*byte).unwrap())
            .unwrap();
        assert_eq!(frame.request_id, 7);
        assert_eq!(frame.challenge, [9; 32]);
        assert!(frame.payload.ends_with(&[0, 0]));
    }

    #[test]
    fn outcome_debug_never_prints_guest_bytes() {
        let outcome = Arm64CommandOutcome {
            stdout: b"secret one".to_vec(),
            stderr: b"secret two".to_vec(),
            terminal: Arm64Terminal::Exited(0),
        };
        assert_eq!(
            format!("{outcome:?}"),
            "Arm64CommandOutcome { stdout_len: 10, stderr_len: 10, terminal: Exited(0) }"
        );
    }
}
