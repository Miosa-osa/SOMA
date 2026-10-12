mod support;

use soma::{
    DirectCommand, Engine, ExecuteMachineRequest, ExecutionLimits, InstanceId,
    LaunchMachineRequest, MachineShape, OciImage, OperationId, OutputChannel, TerminalStatus,
};
use support::{Mode, TestBackend};

fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}

fn launched(mode: Mode) -> (Engine<TestBackend, soma::MemoryStateStore>, InstanceId) {
    let (backend, _calls) = TestBackend::new(mode);
    let mut engine = Engine::new(backend);
    let instance = InstanceId::new("22222222222222222222222222222222").expect("instance");
    engine
        .launch_machine(LaunchMachineRequest::new(
            operation('1'),
            instance.clone(),
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        ))
        .expect("launch succeeds");
    (engine, instance)
}

fn request(instance: &InstanceId, digit: char) -> ExecuteMachineRequest {
    ExecuteMachineRequest::new(
        operation(digit),
        instance.clone(),
        DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
        ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
    )
}

#[test]
fn a_backend_that_cannot_stream_replays_its_output_and_the_receipt_is_the_buffered_one() {
    let (mut engine, instance) = launched(Mode::Happy);
    let mut chunks = Vec::new();
    let streamed = engine
        .execute_machine_streaming(request(&instance, '4'), &mut |channel, bytes| {
            chunks.push((channel, bytes.to_vec()));
        })
        .expect("streams");
    assert_eq!(
        chunks,
        vec![(OutputChannel::Stdout, b"v22.23.2\n".to_vec())]
    );
    assert_eq!(
        streamed.receipt().terminal_status(),
        &TerminalStatus::Exited { code: 0 }
    );

    let (mut other, instance) = launched(Mode::Happy);
    let buffered = other
        .execute_machine(request(&instance, '4'))
        .expect("buffers");
    assert_eq!(
        streamed.output().stdout(),
        buffered.output().stdout(),
        "the same command produces the same retained output"
    );
}

#[test]
fn replaying_a_streamed_operation_runs_nothing_and_streams_nothing() {
    let (mut engine, instance) = launched(Mode::Happy);
    engine
        .execute_machine_streaming(request(&instance, '4'), &mut |_, _| {})
        .expect("first");
    let mut chunks = 0;
    // Output is not retained, so a replay answers with the evidence it kept and no bytes: the
    // same answer a buffered replay gives, and it never reaches the backend or the sink.
    let replay = engine
        .execute_machine_streaming(request(&instance, '4'), &mut |_, _| chunks += 1)
        .expect_err("replay carries no output");
    assert!(matches!(replay, soma::ManagedFailure::ReplayUnavailable(_)));
    assert_eq!(chunks, 0);
}

#[test]
fn a_backend_failure_streams_nothing_and_is_the_same_failure() {
    let (mut engine, instance) = launched(Mode::CommandFailure);
    let mut chunks = 0;
    let failure = engine
        .execute_machine_streaming(request(&instance, '4'), &mut |_, _| chunks += 1)
        .expect_err("fails");
    assert_eq!(chunks, 0);
    let (mut other, instance) = launched(Mode::CommandFailure);
    let buffered = other
        .execute_machine(request(&instance, '4'))
        .expect_err("fails");
    assert_eq!(format!("{failure:?}"), format!("{buffered:?}"));
}
