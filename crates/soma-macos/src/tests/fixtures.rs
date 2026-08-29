use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    ControlLimits, ExecutionLimits, ExecutionStatus, GuestCommand, ImageReference, InstanceId,
    MacOsBackend, MachineShape, OneShotRun, ProcessFailureKind,
    process::{ProcessInvocation, ProcessOutput, ProcessRunner},
};

pub(crate) const INSTANCE: &str = "0123456789abcdef0123456789abcdef";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecordedInvocation {
    pub(crate) program: String,
    pub(crate) arguments: Vec<String>,
    pub(crate) timeout: Duration,
    pub(crate) output_limit: usize,
}

pub(crate) struct ScriptedRunner {
    outcomes: Mutex<VecDeque<Result<ProcessOutput, ProcessFailureKind>>>,
    calls: Mutex<Vec<RecordedInvocation>>,
}

impl ScriptedRunner {
    pub(crate) fn new(
        outcomes: impl IntoIterator<Item = Result<ProcessOutput, ProcessFailureKind>>,
    ) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<RecordedInvocation> {
        self.calls.lock().expect("calls lock").clone()
    }
}

impl ProcessRunner for ScriptedRunner {
    fn run(&self, invocation: &ProcessInvocation) -> Result<ProcessOutput, ProcessFailureKind> {
        self.calls
            .lock()
            .expect("calls lock")
            .push(RecordedInvocation {
                program: invocation.program().display().to_string(),
                arguments: invocation
                    .arguments()
                    .iter()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect(),
                timeout: invocation.timeout(),
                output_limit: invocation.output_limit(),
            });
        self.outcomes
            .lock()
            .expect("outcomes lock")
            .pop_front()
            .expect("every scripted call has an outcome")
    }
}

pub(crate) fn backend(
    outcomes: impl IntoIterator<Item = Result<ProcessOutput, ProcessFailureKind>>,
) -> (MacOsBackend, Arc<ScriptedRunner>) {
    let runner = Arc::new(ScriptedRunner::new(outcomes));
    let backend = MacOsBackend::with_runner(
        "/opt/apple/bin/container",
        Arc::clone(&runner) as Arc<dyn ProcessRunner>,
    );
    (backend, runner)
}

pub(crate) fn output(
    status: ExecutionStatus,
    stdout: impl Into<Vec<u8>>,
    stderr: impl Into<Vec<u8>>,
) -> ProcessOutput {
    output_at(status, stdout, stderr, 17)
}

pub(crate) fn output_at(
    status: ExecutionStatus,
    stdout: impl Into<Vec<u8>>,
    stderr: impl Into<Vec<u8>>,
    elapsed_millis: u64,
) -> ProcessOutput {
    ProcessOutput::new(status, stdout.into(), stderr.into(), elapsed_millis)
}

pub(crate) fn success(stdout: impl Into<Vec<u8>>) -> ProcessOutput {
    output(ExecutionStatus::Exited { code: 0 }, stdout, Vec::new())
}

pub(crate) fn owned_inspection() -> ProcessOutput {
    success(
        format!(
            r#"[{{"configuration":{{"id":"soma-{INSTANCE}","labels":{{"io.miosa.soma.instance":"{INSTANCE}"}},"networks":[{{"network":"default"}}],"resources":{{"cpus":1,"memoryInBytes":1073741824}}}},"id":"soma-{INSTANCE}","status":{{"networks":[{{"network":"default"}}],"state":"running"}}}}]"#
        )
        .into_bytes(),
    )
}

pub(crate) fn instance() -> InstanceId {
    InstanceId::new(INSTANCE).expect("fixture Instance ID")
}

pub(crate) fn execution_limits(output_bytes: u64) -> ExecutionLimits {
    ExecutionLimits::new(5_000, output_bytes).expect("fixture execution limits")
}

pub(crate) fn control_limits() -> ControlLimits {
    ControlLimits::new(5_000, 65_536).expect("fixture control limits")
}

pub(crate) fn node_command() -> GuestCommand {
    GuestCommand::new("/usr/local/bin/node", ["--version"]).expect("fixture guest command")
}

pub(crate) fn one_shot(output_bytes: u64) -> OneShotRun {
    OneShotRun::new(
        instance(),
        ImageReference::new("node:22").expect("fixture image"),
        MachineShape::new(1, 1_073_741_824).expect("fixture shape"),
        node_command(),
        execution_limits(output_bytes),
    )
}

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}
