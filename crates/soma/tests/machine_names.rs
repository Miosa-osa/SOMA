mod support;

use soma::{
    DestroyMachineRequest, Engine, InspectMachineRequest, LaunchMachineRequest, MachineName,
    MachineShape, OciImage, StopMachineRequest,
};
use support::{Mode, TestBackend, run_request};

#[test]
fn machine_names_accept_only_canonical_bounded_metadata() {
    let name = MachineName::parse("agent-22").expect("valid name");
    assert_eq!(name.as_str(), "agent-22");
    assert!(!format!("{name:?}").contains("agent-22"));
    assert_eq!(
        serde_json::from_str::<MachineName>(r#""agent-22""#).expect("roundtrip"),
        name
    );

    for invalid in [
        "",
        "Agent-22",
        "agent_22",
        "-agent",
        "agent-",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(MachineName::parse(invalid).is_err(), "accepted {invalid:?}");
        assert!(
            serde_json::from_str::<MachineName>(&format!(r#""{invalid}""#)).is_err(),
            "deserialized {invalid:?}"
        );
    }
}

#[test]
fn run_name_is_receipted_and_changes_the_request_fingerprint() {
    let (named_backend, _) = TestBackend::new(Mode::Happy);
    let (unnamed_backend, _) = TestBackend::new(Mode::Happy);
    let mut named_engine = Engine::new(named_backend);
    let mut unnamed_engine = Engine::new(unnamed_backend);
    let name = MachineName::parse("benchmark-node").expect("name");

    let named = named_engine
        .run(run_request().with_name(name.clone()))
        .expect("named run");
    let unnamed = unnamed_engine.run(run_request()).expect("unnamed run");

    assert_eq!(named.receipt().machine_name(), Some(&name));
    assert_eq!(unnamed.receipt().machine_name(), None);
    assert_ne!(
        named.receipt().request_fingerprint(),
        unnamed.receipt().request_fingerprint()
    );
}

#[test]
fn managed_name_survives_launch_inspect_and_destroy_receipts() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let instance = soma::InstanceId::new("22222222222222222222222222222222").expect("instance");
    let name = MachineName::parse("named-machine").expect("name");

    let launched = engine
        .launch_machine(
            LaunchMachineRequest::new(
                operation('1'),
                instance.clone(),
                OciImage::parse("node:22").expect("image"),
                MachineShape::new(1, 1_024, 8_192).expect("shape"),
            )
            .with_name(name.clone()),
        )
        .expect("launch");
    let inspected = engine
        .inspect_machine(InspectMachineRequest::new(operation('4'), instance.clone()))
        .expect("inspect");
    let destroyed = engine
        .destroy_machine(DestroyMachineRequest::new(operation('5'), instance))
        .expect("destroy");

    assert_eq!(launched.receipt().machine_name(), Some(&name));
    assert_eq!(inspected.receipt().machine_name(), Some(&name));
    assert_eq!(destroyed.receipt().machine_name(), Some(&name));
}

#[test]
fn duplicate_names_never_route_managed_lifecycle_operations() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let first = soma::InstanceId::new("22222222222222222222222222222222").expect("first");
    let second = soma::InstanceId::new("33333333333333333333333333333333").expect("second");
    let name = MachineName::parse("shared-name").expect("name");

    let launch = |operation_id, instance_id| {
        LaunchMachineRequest::new(
            operation_id,
            instance_id,
            OciImage::parse("node:22").expect("image"),
            MachineShape::new(1, 1_024, 8_192).expect("shape"),
        )
        .with_name(name.clone())
    };
    let first_launch = engine
        .launch_machine(launch(operation('1'), first.clone()))
        .expect("first launch");
    let second_launch = engine
        .launch_machine(launch(operation('2'), second.clone()))
        .expect("second launch");

    engine
        .stop_machine(StopMachineRequest::new(operation('4'), first))
        .expect("stop first by identity");
    let second_inspection = engine
        .inspect_machine(InspectMachineRequest::new(operation('5'), second))
        .expect("second remains addressable by identity");

    assert_ne!(
        first_launch.receipt().request_fingerprint(),
        second_launch.receipt().request_fingerprint()
    );
    assert_eq!(second_inspection.receipt().machine_name(), Some(&name));
}

fn operation(digit: char) -> soma::OperationId {
    soma::OperationId::new(digit.to_string().repeat(32)).expect("operation")
}
