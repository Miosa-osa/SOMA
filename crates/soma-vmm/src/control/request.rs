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
    /// Run one command and send each output chunk back as it arrives, then the final reply.
    ///
    /// This is the one request that gets more than one reply: any number of
    /// [`Reply::Chunk`](super::Reply::Chunk) packets, then exactly one `executed` or `failure`.
    /// It is a word of its own, so a worker that predates it refuses the packet and sends one
    /// `rejected` reply, which the supervisor reads as the command having never run.
    ExecuteStreaming(Execute),
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
            Self::ExecuteStreaming(execute) => encode_execute_in("execute_stream", execute),
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
            "execute_in" => decode::decode_execute_in(&mut tokens).map(Self::Execute),
            "execute_stream" => decode::decode_execute_in(&mut tokens).map(Self::ExecuteStreaming),
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
        return encode_execute_in("execute_in", execute);
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
fn encode_execute_in(word: &str, execute: &Execute) -> String {
    let limits = execute.limits();
    let context = execute.context();
    let mut text = format!(
        "{word} {} {} {} {} {} {}",
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
mod tests;
