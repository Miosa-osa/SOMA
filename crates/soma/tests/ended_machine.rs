//! A sandbox a command took down reads as stopped, can be stopped, and can be created again under
//! its identity (SOMA#220). This is what a desktop whose machine worker was killed looks like
//! after the next command reaches it.

mod support;

use soma::{
    DirectCommand, Engine, ExecuteMachineRequest, ExecutionLimits, InspectMachineRequest,
    InstanceId, LaunchMachineRequest, MachineShape, OciImage, OperationId, StopMachineRequest,
    TerminalStatus,
};
use support::{Mode, TestBackend};

fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}

fn instance() -> InstanceId {
    InstanceId::new("22222222222222222222222222222222").expect("instance")
}

fn launch(digit: char) -> LaunchMachineRequest {
    LaunchMachineRequest::new(
        operation(digit),
        instance(),
        OciImage::parse("node:22").expect("image"),
        MachineShape::new(1, 1_024, 8_192).expect("shape"),
    )
}

/// A sandbox whose first command failed in a way that took its machine down.
fn ended() -> Engine<TestBackend> {
    let (backend, _) = TestBackend::new(Mode::CommandFailure);
    let mut engine = Engine::new(backend);
    engine.launch_machine(launch('1')).expect("launch");
    assert!(
        engine
            .execute_machine(ExecuteMachineRequest::new(
                operation('2'),
                instance(),
                DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("command"),
                ExecutionLimits::new(30_000, 16 * 1024 * 1024).expect("limits"),
            ))
            .is_err(),
        "the command took the machine down"
    );
    engine
}

#[test]
fn a_sandbox_a_command_took_down_reads_as_a_lost_machine() {
    let mut engine = ended();
    let inspection = engine
        .inspect_machine(InspectMachineRequest::new(operation('3'), instance()))
        .expect("an ended sandbox is answered, not refused");
    assert_eq!(format!("{:?}", inspection.state()), "Stopped");
    assert!(inspection.machine_lost());
}

#[test]
fn a_sandbox_a_command_took_down_can_be_stopped_and_created_again() {
    let mut engine = ended();
    let stopped = engine
        .stop_machine(StopMachineRequest::new(operation('3'), instance()))
        .expect("a stop of an ended sandbox is answered, not refused");
    assert_eq!(
        stopped.receipt().terminal_status(),
        &TerminalStatus::Stopped
    );
    stopped
        .receipt()
        .validate()
        .expect("the stop receipt is valid");
    engine
        .stop_machine(StopMachineRequest::new(operation('4'), instance()))
        .expect("a second stop answers like the first");
    engine
        .launch_machine(launch('5'))
        .expect("the identity is created again");
}
