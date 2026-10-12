//! Parsing one control request packet, field by named field.
//!
//! Every value is checked by the contract constructor it feeds rather than by this module, so
//! a request that decodes is always one the Machine may be asked to perform.

use crate::{
    Argument, ContractVersion, DeclaredDevices, DiskBytes, Execute, ExecuteContext,
    ExecutionLimits, Generation, GenerationId, HotplugBytes, InstanceId, Launch, MachineSpec,
    MemoryBytes, OperationId, OutputBytes, Program, Stop, TimeoutMillis, VcpuCount,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::super::PtyRequest;
use super::super::{
    ControlError,
    field::{bytes, identifier, number, untagged},
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
    let contract = ContractVersion::new(number(tokens.next(), "contract")?)
        .map_err(|_| ControlError::InvalidValue("contract"))?;
    let overlay = flag(tokens.next(), "declared overlay")?;
    let network = flag(tokens.next(), "declared network")?;
    let hotplug = tokens
        .next()
        .map(|token| {
            HotplugBytes::new(number(Some(token), "hotplug")?)
                .map_err(|_| ControlError::InvalidValue("hotplug"))
        })
        .transpose()?;
    end(tokens)?;
    let mut machine = MachineSpec::new(vcpus, memory, disk).with_contract(contract);
    if let Some(hotplug) = hotplug {
        machine = machine.with_hotplug(hotplug);
    }
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

/// The most arguments and environment pairs a packet may name, checked before any is read so a
/// hostile count cannot make the decoder allocate for fields the packet does not carry.
const MAX_ARGUMENT_COUNT: usize = 4_096;
const MAX_ENVIRONMENT_COUNT: usize = 64;

pub(super) fn decode_execute_in<'a>(
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
    let program = Program::new(untagged(tokens.next(), "program")?)
        .map_err(|_| ControlError::InvalidValue("program"))?;
    let argument_count: usize = number(tokens.next(), "argument count")?;
    if argument_count > MAX_ARGUMENT_COUNT {
        return Err(ControlError::InvalidValue("argument count"));
    }
    let mut arguments = Vec::with_capacity(argument_count.min(64));
    for _ in 0..argument_count {
        arguments.push(
            Argument::new(untagged(tokens.next(), "argument")?)
                .map_err(|_| ControlError::InvalidValue("argument"))?,
        );
    }
    let environment_count: usize = number(tokens.next(), "environment count")?;
    if environment_count > MAX_ENVIRONMENT_COUNT {
        return Err(ControlError::InvalidValue("environment count"));
    }
    let mut environment = Vec::with_capacity(environment_count);
    for _ in 0..environment_count {
        environment.push((
            untagged(tokens.next(), "environment name")?,
            untagged(tokens.next(), "environment value")?,
        ));
    }
    let mut optional = |field: &'static str| match tokens.next() {
        Some("-") => Ok(None),
        token => untagged(token, field).map(Some),
    };
    let working_directory = optional("working directory")?;
    let user = optional("user")?;
    let stdin = untagged(tokens.next(), "stdin")?;
    end(tokens)?;
    let context = ExecuteContext {
        environment,
        working_directory,
        user,
        stdin,
    };
    // A request that decodes is one the machine may be asked to run, so the guest's own bounds
    // are applied here rather than discovered when the command is built.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    soma_guest::GuestCommand::new(
        program.as_bytes().to_vec(),
        arguments
            .iter()
            .map(|argument| argument.as_bytes().to_vec())
            .collect(),
        timeout.get(),
        output.get(),
    )
    .and_then(|command| command.with_context(context.to_guest()))
    .map_err(|_| ControlError::InvalidValue("context"))?;
    let limits = ExecutionLimits::new(timeout, output);
    Execute::new(operation, instance, program, arguments, limits)
        .map(|execute| Request::Execute(execute.with_context(context)))
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
