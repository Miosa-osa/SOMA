//! The exact bytes of one command body.
//!
//! The body keeps the order the version 1 shape already had, the fixed scalars first and then
//! the program and the arguments, and appends the context after it. Nothing that existed moved,
//! so the layout is still one forward read with no back-patching, but the body is longer than
//! it was: a decoder reads a body to its exact end, and the shorter form is no longer produced
//! by anything, so this is a clean change of the version 1 body rather than an optional tail.
//!
//! An optional field is one presence byte that accepts only zero or one, followed by its field
//! when and only when that byte is one. A decoder that accepted any non-zero byte, or a present
//! flag with an empty field, would give one message several encodings.

use crate::Error;

use super::super::frame::Reader;
use super::{
    CommandContext, GuestCommand, MAX_ARGUMENTS, MAX_ENVIRONMENT, MAX_FIELD_BYTES, MAX_STDIN_BYTES,
    MAX_USER_BYTES,
};

impl GuestCommand {
    pub(in super::super) fn encode_body(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(self.encoded_size());
        body.extend_from_slice(&self.timeout_millis.to_be_bytes());
        body.extend_from_slice(&self.output_bytes.to_be_bytes());
        push_field(&mut body, &self.program);
        push_count(&mut body, self.arguments.len());
        for argument in &self.arguments {
            push_field(&mut body, argument);
        }
        push_count(&mut body, self.environment.len());
        for (name, value) in &self.environment {
            push_field(&mut body, name);
            push_field(&mut body, value);
        }
        push_optional(&mut body, self.working_directory.as_deref());
        push_optional(&mut body, self.user.as_deref());
        push_field(&mut body, &self.stdin);
        body
    }

    pub(in super::super) fn decode_body(body: &[u8]) -> Result<Self, Error> {
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
        let context = decode_context(&mut reader)?;
        reader.finish()?;
        GuestCommand::new(program, arguments, timeout_millis, output_bytes)
            .and_then(|command| command.with_context(context))
            .map_err(|_| Error::ApplicationMessageRejected)
    }
}

/// Reads the environment, working directory, user, and standard input that follow the argument
/// vector.
fn decode_context(reader: &mut Reader<'_>) -> Result<CommandContext, Error> {
    let environment_count = usize::from(reader.u16()?);
    if environment_count > MAX_ENVIRONMENT {
        return Err(Error::ApplicationMessageRejected);
    }
    let mut environment = Vec::with_capacity(environment_count);
    for _ in 0..environment_count {
        let name = reader.field(MAX_FIELD_BYTES)?.to_vec();
        let value = reader.field(MAX_FIELD_BYTES)?.to_vec();
        environment.push((name, value));
    }
    Ok(CommandContext {
        environment,
        working_directory: read_optional(reader, MAX_FIELD_BYTES)?,
        user: read_optional(reader, MAX_USER_BYTES)?,
        stdin: reader.field(MAX_STDIN_BYTES)?.to_vec(),
    })
}

/// Reads one presence byte and the field that follows it when it is set.
///
/// A present field must be nonempty, because an absent field already encodes emptiness and two
/// encodings of one message is what this codec refuses to admit.
fn read_optional(reader: &mut Reader<'_>, maximum: usize) -> Result<Option<Vec<u8>>, Error> {
    match reader.u8()? {
        0 => Ok(None),
        1 => {
            let field = reader.field(maximum)?;
            if field.is_empty() {
                return Err(Error::ApplicationMessageRejected);
            }
            Ok(Some(field.to_vec()))
        }
        _ => Err(Error::ApplicationMessageRejected),
    }
}

fn push_optional(body: &mut Vec<u8>, field: Option<&[u8]>) {
    match field {
        None => body.push(0),
        Some(bytes) => {
            body.push(1);
            push_field(body, bytes);
        }
    }
}

fn push_count(body: &mut Vec<u8>, count: usize) {
    let count = u16::try_from(count).expect("a validated element count");
    body.extend_from_slice(&count.to_be_bytes());
}

/// Writes one length-prefixed field, using the same `u16` prefix every field on this protocol
/// uses.
///
/// Every caller writes a field the constructor has already bounded below `u16::MAX`, so a
/// longer one is a caller bug rather than a wire condition, and clamping would silently shorten
/// the field instead of failing.
fn push_field(body: &mut Vec<u8>, field: &[u8]) {
    let length = u16::try_from(field.len()).expect("a validated field length");
    body.extend_from_slice(&length.to_be_bytes());
    body.extend_from_slice(field);
}
