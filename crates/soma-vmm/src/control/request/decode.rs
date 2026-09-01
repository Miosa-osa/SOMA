//! Parsing one control request packet, field by named field.
//!
//! Every value is checked by the contract constructor it feeds rather than by this module, so
//! a request that decodes is always one the Machine may be asked to perform.

use crate::{
    Argument, DeclaredDevices, DiskBytes, Execute, ExecutionLimits, Generation, GenerationId,
    InstanceId, Launch, MachineSpec, MemoryBytes, OperationId, OutputBytes, Program, Stop,
    TimeoutMillis, VcpuCount,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::super::PtyRequest;
use super::super::{
    ControlError,
    field::{bytes, identifier, number},
    window::{OutputStream, OutputWindow},
};
use super::Request;

pub(super) use super::super::field::end;

pub(super) fn decode_shutdown<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let status = number(tokens.next(), "exit status")?;
    end(tokens).map(|()| Request::Shutdown(status))
}

pub(super) fn decode_launch<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let instance = InstanceId::new(identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    let generation = GenerationId::new(identifier(tokens.next(), "generation")?)
        .map_err(|_| ControlError::InvalidValue("generation"))?;
    let vcpus = VcpuCount::new(number(tokens.next(), "vcpus")?)
        .map_err(|_| ControlError::InvalidValue("vcpus"))?;
    let memory = MemoryBytes::new(number(tokens.next(), "memory")?)
        .map_err(|_| ControlError::InvalidValue("memory"))?;
    let disk = DiskBytes::new(number(tokens.next(), "disk")?)
        .map_err(|_| ControlError::InvalidValue("disk"))?;
    let overlay = flag(tokens.next(), "declared overlay")?;
    let network = flag(tokens.next(), "declared network")?;
    end(tokens)?;
    let machine = MachineSpec::new(vcpus, memory, disk);
    let devices = DeclaredDevices::new(overlay, network);
    Ok(Request::Launch(Launch::new(
        operation,
        instance,
        Generation::new(generation, machine, devices),
    )))
}

/// Decodes a declaration flag, which is written as a digit so the packet stays one line of
/// unambiguous ASCII fields.
fn flag(token: Option<&str>, field: &'static str) -> Result<bool, ControlError> {
    match number::<u8>(token, field)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ControlError::InvalidValue(field)),
    }
}

pub(super) fn decode_output<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let stream = tokens
        .next()
        .and_then(OutputStream::from_token)
        .ok_or(ControlError::InvalidValue("stream"))?;
    let offset = number(tokens.next(), "offset")?;
    let length = number(tokens.next(), "length")?;
    end(tokens)?;
    OutputWindow::new(operation, stream, offset, length)
        .map(Request::Output)
        .ok_or(ControlError::InvalidValue("length"))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn decode_pty<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let instance = InstanceId::new(identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    let encoded = bytes(tokens.next(), "terminal operation")?;
    end(tokens)?;
    let terminal = serde_json::from_slice::<soma::PtyOperation>(&encoded)
        .map_err(|_| ControlError::InvalidValue("terminal operation"))?;
    terminal
        .check()
        .map_err(|_| ControlError::InvalidValue("terminal operation"))?;
    Ok(Request::Pty(PtyRequest::new(operation, instance, terminal)))
}

pub(super) fn decode_execute<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let instance = InstanceId::new(identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    let timeout = TimeoutMillis::new(number(tokens.next(), "timeout")?)
        .map_err(|_| ControlError::InvalidValue("timeout"))?;
    let output = OutputBytes::new(number(tokens.next(), "output")?)
        .map_err(|_| ControlError::InvalidValue("output"))?;
    let program = Program::new(bytes(tokens.next(), "program")?)
        .map_err(|_| ControlError::InvalidValue("program"))?;
    let mut arguments = Vec::new();
    for token in tokens.by_ref() {
        let argument = Argument::new(bytes(Some(token), "argument")?)
            .map_err(|_| ControlError::InvalidValue("argument"))?;
        arguments.push(argument);
    }
    let limits = ExecutionLimits::new(timeout, output);
    Execute::new(operation, instance, program, arguments, limits)
        .map(Request::Execute)
        .map_err(|_| ControlError::InvalidValue("arguments"))
}

pub(super) fn decode_stop<'a>(
    tokens: &mut impl Iterator<Item = &'a str>,
) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let instance = InstanceId::new(identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    end(tokens)?;
    Ok(Request::Stop(Stop::new(operation, instance)))
}
