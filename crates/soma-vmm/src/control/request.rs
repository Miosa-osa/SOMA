use crate::{
    Argument, DiskBytes, Execute, ExecutionLimits, Generation, GenerationId, InstanceId, Launch,
    MachineSpec, MemoryBytes, OperationId, OutputBytes, Program, Stop, TimeoutMillis, VcpuCount,
};

use super::{
    ControlError,
    field::{bytes, end, hex, identifier, number},
};

/// The largest control packet a worker accepts.
///
/// One `SOCK_SEQPACKET` datagram carries one request, so the bound is also the worker's
/// receive buffer: a longer packet is refused rather than silently truncated.
pub const MAX_REQUEST_BYTES: usize = 4096;

/// One request packet a supervisor sends to a jailed worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Observe containment again and reply with the attestation.
    ///
    /// The observation needs the startup-only syscalls that enumerate the root, so this
    /// request is admitted before [`Request::Seal`] and is a seccomp kill after it.
    Attest,
    Launch(Launch),
    Execute(Execute),
    Stop(Stop),
    /// Narrow the seccomp filter to its steady-state phase.
    Seal,
    /// Leave with this exit status.
    Shutdown(i32),
}

impl Request {
    #[must_use]
    pub fn encode(&self) -> String {
        match self {
            Self::Attest => "attest".to_owned(),
            Self::Launch(launch) => encode_launch(launch),
            Self::Execute(execute) => encode_execute(execute),
            Self::Stop(stop) => format!(
                "stop {} {}",
                hex(stop.operation_id().as_bytes()),
                hex(stop.instance_id().as_bytes())
            ),
            Self::Seal => "seal".to_owned(),
            Self::Shutdown(status) => format!("shutdown {status}"),
        }
    }

    /// Parses one request packet.
    ///
    /// # Errors
    ///
    /// Returns the [`ControlError`] naming the first field the packet does not satisfy; every
    /// value is checked by the contract constructor it feeds, so a decoded request is always
    /// one the Machine may be asked to perform.
    pub fn decode(text: &str) -> Result<Self, ControlError> {
        if text.len() > MAX_REQUEST_BYTES {
            return Err(ControlError::TooLong);
        }
        let mut tokens = text.split_whitespace();
        match tokens.next().unwrap_or_default() {
            "attest" => end(tokens).map(|()| Self::Attest),
            "seal" => end(tokens).map(|()| Self::Seal),
            "shutdown" => {
                let status = number(tokens.next(), "exit status")?;
                end(tokens).map(|()| Self::Shutdown(status))
            }
            "launch" => decode_launch(&mut tokens),
            "execute" => decode_execute(&mut tokens),
            "stop" => decode_stop(&mut tokens),
            _ => Err(ControlError::UnknownRequest),
        }
    }
}

fn encode_launch(launch: &Launch) -> String {
    let machine = launch.generation().machine();
    format!(
        "launch {} {} {} {} {} {}",
        hex(launch.operation_id().as_bytes()),
        hex(launch.instance_id().as_bytes()),
        hex(launch.generation().id().as_bytes()),
        machine.vcpus().get(),
        machine.memory().get(),
        machine.writable_disk().get(),
    )
}

fn encode_execute(execute: &Execute) -> String {
    let limits = execute.limits();
    let mut text = format!(
        "execute {} {} {} {} {}",
        hex(execute.operation_id().as_bytes()),
        hex(execute.instance_id().as_bytes()),
        limits.timeout().get(),
        limits.output().get(),
        hex(execute.program().as_bytes()),
    );
    for argument in execute.arguments() {
        text.push(' ');
        text.push_str(&hex(argument.as_bytes()));
    }
    text
}

fn decode_launch<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Result<Request, ControlError> {
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
    end(tokens)?;
    let machine = MachineSpec::new(vcpus, memory, disk);
    Ok(Request::Launch(Launch::new(
        operation,
        instance,
        Generation::new(generation, machine),
    )))
}

fn decode_execute<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Result<Request, ControlError> {
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

fn decode_stop<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Result<Request, ControlError> {
    let operation = OperationId::new(identifier(tokens.next(), "operation")?)
        .map_err(|_| ControlError::InvalidValue("operation"))?;
    let instance = InstanceId::new(identifier(tokens.next(), "instance")?)
        .map_err(|_| ControlError::InvalidValue("instance"))?;
    end(tokens)?;
    Ok(Request::Stop(Stop::new(operation, instance)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation() -> OperationId {
        OperationId::new([1; 16]).expect("operation")
    }

    fn instance() -> InstanceId {
        InstanceId::new([2; 16]).expect("instance")
    }

    fn launch() -> Launch {
        let machine = MachineSpec::new(
            VcpuCount::new(1).expect("vcpus"),
            MemoryBytes::new(1 << 30).expect("memory"),
            DiskBytes::new(1 << 32).expect("disk"),
        );
        let generation = Generation::new(GenerationId::new([3; 32]).expect("generation"), machine);
        Launch::new(operation(), instance(), generation)
    }

    fn execute() -> Execute {
        let limits = ExecutionLimits::new(
            TimeoutMillis::new(5_000).expect("timeout"),
            OutputBytes::new(65_536).expect("output"),
        );
        Execute::new(
            operation(),
            instance(),
            Program::new(b"/bin/node".to_vec()).expect("program"),
            vec![Argument::new(b"--version".to_vec()).expect("argument")],
            limits,
        )
        .expect("execute")
    }

    #[test]
    fn every_request_round_trips() {
        let requests = [
            Request::Attest,
            Request::Seal,
            Request::Shutdown(-1),
            Request::Launch(launch()),
            Request::Execute(execute()),
            Request::Stop(Stop::new(operation(), instance())),
        ];
        for request in requests {
            let encoded = request.encode();
            assert!(encoded.is_ascii() && !encoded.contains('\n'), "{encoded:?}");
            assert_eq!(Request::decode(&encoded), Ok(request), "{encoded:?}");
        }
    }

    #[test]
    fn decoding_names_the_field_it_refuses() {
        assert_eq!(Request::decode("mount"), Err(ControlError::UnknownRequest));
        assert_eq!(
            Request::decode("attest now"),
            Err(ControlError::TrailingField)
        );
        assert_eq!(
            Request::decode("shutdown"),
            Err(ControlError::MissingField("exit status"))
        );
        assert_eq!(
            Request::decode(&"seal ".repeat(MAX_REQUEST_BYTES)),
            Err(ControlError::TooLong)
        );
        let zero_operation = Request::Stop(Stop::new(operation(), instance()))
            .encode()
            .replace(&hex(&[1; 16]), &hex(&[0; 16]));
        assert_eq!(
            Request::decode(&zero_operation),
            Err(ControlError::InvalidValue("operation"))
        );
    }

    #[test]
    fn a_relative_program_is_refused_by_the_contract_type() {
        let relative = format!(
            "execute {} {} 1 1 {}",
            hex(&[1; 16]),
            hex(&[2; 16]),
            hex(b"node")
        );
        assert_eq!(
            Request::decode(&relative),
            Err(ControlError::InvalidValue("program"))
        );
    }
}
