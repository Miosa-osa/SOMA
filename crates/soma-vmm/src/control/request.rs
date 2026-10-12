use crate::{Execute, Launch, Stop};

use super::{
    ControlError,
    field::{hex, tagged},
    window::OutputWindow,
};

mod decode;

use decode::{decode_execute, decode_launch, decode_output, decode_stop};

/// The largest control packet a worker accepts.
///
/// One `SOCK_SEQPACKET` datagram carries one request, so the bound is also the worker's
/// receive buffer: a longer packet is refused rather than silently truncated.
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

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
    /// Perform one bounded terminal exchange against the live guest session.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    Pty(super::PtyRequest),
    Stop(Stop),
    /// Narrow the seccomp filter to its steady-state phase.
    Seal,
    /// Leave with this exit status.
    Shutdown(i32),
}

impl Request {
    /// Encodes one supervisor request into its bounded textual packet.
    ///
    /// # Panics
    ///
    /// Panics only if the typed terminal operation cannot be serialized by `serde_json`.
    /// Every field in that closed contract is serializable, so this indicates a programming
    /// error introduced alongside a future contract change rather than hostile input.
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
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            Self::Pty(request) => format!(
                "pty {} {} {}",
                hex(request.operation_id().as_bytes()),
                hex(request.instance_id().as_bytes()),
                hex(&serde_json::to_vec(request.operation())
                    .expect("terminal operation serializes")),
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
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            "pty" => decode::decode_pty(&mut tokens),
            "execute" => decode_execute(&mut tokens),
            "execute_in" => decode::decode_execute_in(&mut tokens),
            "stop" => decode_stop(&mut tokens),
            _ => Err(ControlError::UnknownRequest),
        }
    }
}

fn encode_launch(launch: &Launch) -> String {
    let machine = launch.generation().machine();
    let devices = launch.generation().devices();
    let mut text = format!(
        "launch {} {} {} {} {} {} {} {} {}",
        hex(launch.operation_id().as_bytes()),
        hex(launch.instance_id().as_bytes()),
        hex(launch.generation().id().as_bytes()),
        machine.vcpus().get(),
        machine.memory().get(),
        machine.writable_disk().get(),
        machine.contract().get(),
        u8::from(devices.writable_disk()),
        u8::from(devices.network()),
    );
    // A machine with no resizable region encodes exactly as every earlier launch did.
    if let Some(hotplug) = machine.hotplug() {
        text.push(' ');
        text.push_str(&hotplug.get().to_string());
    }
    text
}

fn encode_execute(execute: &Execute) -> String {
    if !execute.context().is_empty() {
        return encode_execute_in(execute);
    }
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

/// A command with a context, in a form of its own.
///
/// A worker built before contexts existed knows only `execute`, so it refuses this word and the
/// command never runs without the environment the caller asked for. The plain form above is
/// untouched, so an older worker keeps serving every command that has no context.
///
/// Layout: operation, instance, timeout, output, program, argument count and arguments,
/// environment count and name/value pairs, working directory, user, standard input. Every byte
/// string is `x`-prefixed hexadecimal, and an absent directory or user is `-`.
fn encode_execute_in(execute: &Execute) -> String {
    let limits = execute.limits();
    let context = execute.context();
    let mut text = format!(
        "execute_in {} {} {} {} {} {}",
        hex(execute.operation_id().as_bytes()),
        hex(execute.instance_id().as_bytes()),
        limits.timeout().get(),
        limits.output().get(),
        tagged(execute.program().as_bytes()),
        execute.arguments().len(),
    );
    for argument in execute.arguments() {
        text.push(' ');
        text.push_str(&tagged(argument.as_bytes()));
    }
    text.push(' ');
    text.push_str(&context.environment.len().to_string());
    for (name, value) in &context.environment {
        text.push(' ');
        text.push_str(&tagged(name));
        text.push(' ');
        text.push_str(&tagged(value));
    }
    for optional in [&context.working_directory, &context.user] {
        text.push(' ');
        match optional {
            Some(bytes) => text.push_str(&tagged(bytes)),
            None => text.push('-'),
        }
    }
    text.push(' ');
    text.push_str(&tagged(&context.stdin));
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

    fn launch_with_hotplug(bytes: u64) -> Launch {
        let machine = MachineSpec::new(
            VcpuCount::new(1).expect("vcpus"),
            MemoryBytes::new(1 << 30).expect("memory"),
            DiskBytes::new(1 << 32).expect("disk"),
        )
        .with_hotplug(crate::HotplugBytes::new(bytes).expect("hotplug"));
        Launch::new(
            operation(),
            instance(),
            Generation::new(
                GenerationId::new([3; 32]).expect("generation"),
                machine,
                DeclaredDevices::new(true, true),
            ),
        )
    }

    #[test]
    fn a_launch_without_a_region_encodes_exactly_as_before_and_one_with_it_round_trips() {
        let plain = Request::Launch(launch()).encode();
        assert_eq!(plain.split(' ').count(), 10, "{plain}");
        let region = Request::Launch(launch_with_hotplug(1 << 30));
        let encoded = region.encode();
        assert_eq!(encoded, format!("{plain} {}", 1_u64 << 30));
        assert_eq!(Request::decode(&encoded), Ok(region));
        assert!(
            matches!(
                Request::decode(&Request::Launch(launch()).encode()),
                Ok(Request::Launch(decoded)) if decoded.generation().machine().hotplug().is_none()
            ),
            "the old form decodes with no region"
        );
    }

    #[test]
    fn a_malformed_or_extra_hotplug_field_is_refused() {
        let plain = Request::Launch(launch()).encode();
        for bad in [
            format!("{plain} 0"),
            format!("{plain} x"),
            format!("{plain} -1"),
            format!("{plain} 1024 1024"),
        ] {
            assert!(Request::decode(&bad).is_err(), "{bad}");
        }
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

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn terminal_requests_round_trip_binary_input_without_logging_it() {
        let request = Request::Pty(super::super::PtyRequest::new(
            operation(),
            instance(),
            soma::PtyOperation::Write {
                bytes: vec![0, 255, b'\n'],
            },
        ));
        let encoded = request.encode();
        assert!(encoded.is_ascii() && !encoded.contains('\n'));
        assert_eq!(Request::decode(&encoded), Ok(request));
    }
    fn context() -> crate::ExecuteContext {
        crate::ExecuteContext {
            environment: vec![
                (b"A".to_vec(), b"1".to_vec()),
                (b"EMPTY".to_vec(), Vec::new()),
                (b"SPACED".to_vec(), b"two words".to_vec()),
            ],
            working_directory: Some(b"/work dir".to_vec()),
            user: None,
            stdin: b"SECRETSTDIN".to_vec(),
        }
    }

    #[test]
    fn a_command_without_a_context_still_encodes_as_the_original_word() {
        let text = Request::Execute(execute()).encode();
        assert!(text.starts_with("execute "));
        assert!(!text.contains("execute_in"));
        let plain = Request::decode(&text).expect("decodes");
        assert_eq!(plain, Request::Execute(execute()));
    }

    #[test]
    fn a_context_round_trips_including_empty_values_and_spaces() {
        let request = Request::Execute(execute().with_context(context()));
        let text = request.encode();
        assert!(text.starts_with("execute_in "));
        assert!(text.len() < MAX_REQUEST_BYTES);
        assert_eq!(Request::decode(&text).expect("decodes"), request);
    }

    #[test]
    fn an_optional_user_and_directory_round_trip_when_present_and_absent() {
        for (directory, user) in [
            (None, None),
            (Some(b"/w".to_vec()), Some(b"dev".to_vec())),
            (None, Some(b"dev".to_vec())),
        ] {
            let mut context = context();
            context.working_directory = directory;
            context.user = user;
            let request = Request::Execute(execute().with_context(context));
            assert_eq!(
                Request::decode(&request.encode()).expect("decodes"),
                request
            );
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn a_context_the_guest_would_refuse_is_refused_at_decode() {
        for mutate in [
            (|c: &mut crate::ExecuteContext| c.working_directory = Some(b"relative".to_vec()))
                as fn(&mut crate::ExecuteContext),
            |c| c.environment.push((b"A=B".to_vec(), b"v".to_vec())),
            |c| c.environment.push((Vec::new(), b"v".to_vec())),
            |c| c.stdin = vec![0; 32 * 1024 + 1],
            |c| c.user = Some(Vec::new()),
        ] {
            let mut bad = context();
            mutate(&mut bad);
            let text = Request::Execute(execute().with_context(bad)).encode();
            if text.len() <= MAX_REQUEST_BYTES {
                assert_eq!(
                    Request::decode(&text),
                    Err(ControlError::InvalidValue("context")),
                );
            }
        }
    }

    #[test]
    fn hostile_counts_are_refused_before_any_allocation_for_them() {
        let head = format!(
            "execute_in {} {} 5000 65536 x2f62696e2f74727565",
            hex(operation().as_bytes()),
            hex(instance().as_bytes())
        );
        assert_eq!(
            Request::decode(&format!("{head} 4294967295")),
            Err(ControlError::InvalidValue("argument count"))
        );
        assert_eq!(
            Request::decode(&format!("{head} 0 65")),
            Err(ControlError::InvalidValue("environment count"))
        );
        assert_eq!(
            Request::decode(&format!("{head} 0 1 x41")),
            Err(ControlError::MissingField("environment value"))
        );
        assert_eq!(
            Request::decode(&format!("{head} 0 0 - - x extra")),
            Err(ControlError::TrailingField)
        );
        // An untagged field is refused rather than read as hexadecimal.
        assert_eq!(
            Request::decode(&format!("{head} 0 0 - - 6162")),
            Err(ControlError::InvalidValue("stdin"))
        );
    }

    #[test]
    fn debug_never_prints_a_value() {
        let shown = format!("{:?}", execute().with_context(context()));
        assert!(!shown.contains("two words") && !shown.contains("SECRETSTDIN"));
    }
}
