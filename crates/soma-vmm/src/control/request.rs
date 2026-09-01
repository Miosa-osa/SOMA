use crate::{Execute, Launch, Stop};

use super::{ControlError, field::hex, window::OutputWindow};

mod decode;

use decode::{decode_execute, decode_launch, decode_output, decode_stop};

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
    /// Read one bounded window of a completed command's output.
    ///
    /// A reply packet cannot carry sixteen mebibytes, so the output an Execute produced stays
    /// in the worker's own operation receipt and the supervisor reads it back one window at a
    /// time. Nothing is recomputed: the windows come out of the receipt the Execute already
    /// returned, so a supervisor that reads it twice reads the same bytes.
    Output(OutputWindow),
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
            Self::Output(window) => format!(
                "output {} {} {} {}",
                hex(window.operation_id().as_bytes()),
                window.stream().token(),
                window.offset(),
                window.length(),
            ),
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
            "attest" => decode::end(tokens).map(|()| Self::Attest),
            "seal" => decode::end(tokens).map(|()| Self::Seal),
            "shutdown" => decode::decode_shutdown(&mut tokens),
            "launch" => decode_launch(&mut tokens),
            "output" => decode_output(&mut tokens),
            "execute" => decode_execute(&mut tokens),
            "stop" => decode_stop(&mut tokens),
            _ => Err(ControlError::UnknownRequest),
        }
    }
}

fn encode_launch(launch: &Launch) -> String {
    let machine = launch.generation().machine();
    let devices = launch.generation().devices();
    format!(
        "launch {} {} {} {} {} {} {} {}",
        hex(launch.operation_id().as_bytes()),
        hex(launch.instance_id().as_bytes()),
        hex(launch.generation().id().as_bytes()),
        machine.vcpus().get(),
        machine.memory().get(),
        machine.writable_disk().get(),
        u8::from(devices.writable_disk()),
        u8::from(devices.network()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeclaredDevices;
    use crate::control::OutputStream;
    use crate::{
        Argument, DiskBytes, ExecutionLimits, Generation, GenerationId, InstanceId, MachineSpec,
        MemoryBytes, OperationId, OutputBytes, Program, TimeoutMillis, VcpuCount,
    };

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
        let generation = Generation::new(
            GenerationId::new([3; 32]).expect("generation"),
            machine,
            DeclaredDevices::new(true, true),
        );
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
            Request::Output(
                OutputWindow::new(operation(), OutputStream::Stderr, 1024, 4096).expect("window"),
            ),
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
