use soma::{
    Capabilities, Engine, InstanceId, LaunchMachineRequest, MachineName, MachineShape,
    ManagedFailure, ManagedStateError, MemoryStateStore, OciImage, OperationId,
};

use crate::{
    fixtures::{instance, launch_request, operation},
    support::{Mode, TestBackend},
};

/// The fixture launch, with the network a platform sandbox runs with.
fn networked(operation_id: OperationId, instance_id: InstanceId) -> LaunchMachineRequest {
    LaunchMachineRequest::new(
        operation_id,
        instance_id,
        OciImage::parse("node:22").expect("image"),
        MachineShape::new(1, 1_024, 8_192)
            .expect("shape")
            .with_capabilities(Capabilities::unspecified().with_network_access()),
    )
}

fn child() -> InstanceId {
    InstanceId::new("33333333333333333333333333333333").expect("child")
}

#[test]
fn a_fork_of_a_networked_sandbox_keeps_its_network() {
    let (backend, calls) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, MemoryStateStore::new());
    let owner = MachineName::parse("tenant-a").expect("owner");
    let source = networked(operation('1'), instance()).with_name(owner.clone());
    engine
        .launch_machine(source.clone())
        .expect("launch source");

    let fork = networked(operation('2'), child()).with_name(owner);
    engine
        .fork_machine(fork, &instance(), &operation('a'))
        .expect("a fork asks for the same network its source runs with");

    assert_eq!(
        calls
            .lock()
            .expect("call log")
            .iter()
            .filter(|call| **call == "launch_checkpoint")
            .count(),
        1,
    );
}

#[test]
fn a_fork_still_refuses_a_shape_its_source_does_not_run() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::with_state_store(backend, MemoryStateStore::new());
    let owner = MachineName::parse("tenant-a").expect("owner");
    let source = networked(operation('1'), instance()).with_name(owner.clone());
    engine.launch_machine(source).expect("launch source");

    let isolated = launch_request(operation('2'), child()).with_name(owner);
    assert_eq!(
        engine
            .fork_machine(isolated, &instance(), &operation('a'))
            .expect_err("the fork's shape differs from its source"),
        ManagedFailure::State(ManagedStateError::OperationConflict),
    );
}
