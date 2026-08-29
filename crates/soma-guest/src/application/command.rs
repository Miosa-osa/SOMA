use core::fmt;

use crate::Error;

use super::{MAX_BODY_SIZE, frame::Reader};

/// Maximum number of direct arguments in one authenticated command.
pub const MAX_ARGUMENTS: usize = 64;
/// Maximum byte length of the program or one argument.
pub const MAX_FIELD_BYTES: usize = 4096;
/// Maximum command timeout represented by the protocol.
pub const MAX_TIMEOUT_MILLIS: u32 = 3_600_000;
/// Maximum combined output allowance represented by the protocol.
pub const MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

const FIXED_BODY_SIZE: usize = 4 + 8 + 2 + 2;
const PROBE_ARGUMENT: &[u8] = b"--soma-ready-probe-v1";
const PROBE_PROGRAM: &[u8] = b"/proc/self/exe";
const PROBE_TIMEOUT_MILLIS: u32 = 1_000;
const PROBE_OUTPUT_BYTES: u64 = 1;

/// A shell-free, bounded direct guest process invocation.
#[derive(Clone, Eq, PartialEq)]
pub struct GuestCommand {
    program: Box<[u8]>,
    arguments: Box<[Box<[u8]>]>,
    timeout_millis: u32,
    output_bytes: u64,
}

impl GuestCommand {
    pub(crate) fn readiness_probe() -> Self {
        Self::new(
            PROBE_PROGRAM.to_vec(),
            vec![PROBE_ARGUMENT.to_vec()],
            PROBE_TIMEOUT_MILLIS,
            PROBE_OUTPUT_BYTES,
        )
        .expect("fixed readiness probe satisfies the wire contract")
    }

    /// Creates a command that fits in one authenticated application message.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidCommand`] for invalid paths, NUL, count, field, limit, or aggregate
    /// bounds.
    pub fn new(
        program: Vec<u8>,
        arguments: Vec<Vec<u8>>,
        timeout_millis: u32,
        output_bytes: u64,
    ) -> Result<Self, Error> {
        validate_field(&program, false)?;
        if !program.starts_with(b"/")
            || arguments.len() > MAX_ARGUMENTS
            || timeout_millis == 0
            || timeout_millis > MAX_TIMEOUT_MILLIS
            || output_bytes == 0
            || output_bytes > MAX_OUTPUT_BYTES
        {
            return Err(Error::InvalidCommand);
        }
        let mut encoded_size = FIXED_BODY_SIZE
            .checked_add(program.len())
            .ok_or(Error::InvalidCommand)?;
        for argument in &arguments {
            validate_field(argument, true)?;
            encoded_size = encoded_size
                .checked_add(2)
                .and_then(|size| size.checked_add(argument.len()))
                .ok_or(Error::InvalidCommand)?;
        }
        if encoded_size > MAX_BODY_SIZE {
            return Err(Error::InvalidCommand);
        }
        Ok(Self {
            program: program.into_boxed_slice(),
            arguments: arguments.into_iter().map(Vec::into_boxed_slice).collect(),
            timeout_millis,
            output_bytes,
        })
    }

    /// Returns the exact executable path bytes.
    #[must_use]
    pub fn program(&self) -> &[u8] {
        &self.program
    }

    /// Returns the exact shell-free argument byte strings.
    #[must_use]
    pub fn arguments(&self) -> &[Box<[u8]>] {
        &self.arguments
    }

    /// Returns the command timeout in whole milliseconds.
    #[must_use]
    pub const fn timeout_millis(&self) -> u32 {
        self.timeout_millis
    }

    /// Returns the combined stdout and stderr allowance.
    #[must_use]
    pub const fn output_bytes(&self) -> u64 {
        self.output_bytes
    }

    pub(super) fn encode_body(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.encoded_size());
        body.extend_from_slice(&self.timeout_millis.to_be_bytes());
        body.extend_from_slice(&self.output_bytes.to_be_bytes());
        push_field(&mut body, &self.program);
        body.extend_from_slice(
            &u16::try_from(self.arguments.len())
                .expect("validated argument count")
                .to_be_bytes(),
        );
        for argument in &self.arguments {
            push_field(&mut body, argument);
        }
        body
    }

    pub(super) fn decode_body(body: &[u8]) -> Result<Self, Error> {
        let mut reader = Reader::new(body);
        let timeout_millis = reader.u32()?;
        let output_bytes = reader.u64()?;
        let program = reader.field(MAX_FIELD_BYTES)?.to_vec();
        let argument_count = usize::from(reader.u16()?);
        if argument_count > MAX_ARGUMENTS {
            return Err(Error::ApplicationMessageRejected);
        }
        let mut arguments = Vec::with_capacity(argument_count);
        for _ in 0..argument_count {
            arguments.push(reader.field(MAX_FIELD_BYTES)?.to_vec());
        }
        reader.finish()?;
        Self::new(program, arguments, timeout_millis, output_bytes)
            .map_err(|_| Error::ApplicationMessageRejected)
    }

    fn encoded_size(&self) -> usize {
        FIXED_BODY_SIZE
            + self.program.len()
            + self
                .arguments
                .iter()
                .map(|argument| 2 + argument.len())
                .sum::<usize>()
    }
}

impl fmt::Debug for GuestCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestCommand")
            .field("program_bytes", &self.program.len())
            .field("argument_count", &self.arguments.len())
            .field("timeout_millis", &self.timeout_millis)
            .field("output_bytes", &self.output_bytes)
            .finish()
    }
}

fn validate_field(field: &[u8], empty_allowed: bool) -> Result<(), Error> {
    if (!empty_allowed && field.is_empty()) || field.len() > MAX_FIELD_BYTES || field.contains(&0) {
        return Err(Error::InvalidCommand);
    }
    Ok(())
}

fn push_field(destination: &mut Vec<u8>, field: &[u8]) {
    destination.extend_from_slice(
        &u16::try_from(field.len())
            .expect("validated field length")
            .to_be_bytes(),
    );
    destination.extend_from_slice(field);
}
