mod support;

use soma::{
    Engine, ExecutionReceipt, InstanceId, LaunchMachineRequest, MachineShape, OciImage,
    OperationId, StopMachineRequest,
};
use support::{Mode, TestBackend, run_request};

#[test]
fn receipt_encoding_and_fingerprint_are_deterministic_and_round_trip() {
    let (first_backend, _) = TestBackend::new(Mode::Happy);
    let (second_backend, _) = TestBackend::new(Mode::Happy);
    let mut first_engine = Engine::new(first_backend);
    let mut second_engine = Engine::new(second_backend);

    let first = first_engine.run(run_request()).expect("first run succeeds");
    let second = second_engine
        .run(run_request())
        .expect("second run succeeds");
    let first_json = serde_json::to_string(first.receipt()).expect("receipt encodes");
    let second_json = serde_json::to_string(second.receipt()).expect("receipt encodes");
    let decoded: ExecutionReceipt = serde_json::from_str(&first_json).expect("receipt decodes");

    assert_eq!(first_json, second_json);
    assert_eq!(
        first.receipt().request_fingerprint(),
        second.receipt().request_fingerprint()
    );
    assert_eq!(&decoded, first.receipt());
    decoded.validate().expect("decoded receipt is valid");
}

#[test]
fn receipt_and_debug_output_do_not_reveal_request_content() {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let outcome = engine.run(run_request()).expect("run succeeds");

    let encoded = serde_json::to_string(outcome.receipt()).expect("receipt encodes");
    let debug = format!("{outcome:?}");

    for secret in ["node:22", "/usr/local/bin/node", "--version"] {
        assert!(!encoded.contains(secret));
        assert!(!debug.contains(secret));
    }
}

#[test]
fn portable_engine_is_send_when_its_backend_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Engine<TestBackend>>();
}

#[test]
fn receipt_decoder_rejects_cross_field_and_sequence_forgeries() {
    let run = valid_run_receipt();
    let launch = valid_launch_receipt();
    let stop = valid_stop_receipt();

    let mut ready_without_ready = launch.clone();
    ready_without_ready["milestones"]
        .as_array_mut()
        .expect("milestones")
        .retain(|value| value["kind"] != "ready");
    reject(ready_without_ready);

    let mut ready_without_admission = launch.clone();
    ready_without_admission["milestones"]
        .as_array_mut()
        .expect("milestones")
        .retain(|value| value["kind"] != "admitted");
    reject(ready_without_admission);

    let mut run_without_ready = run.clone();
    run_without_ready["milestones"]
        .as_array_mut()
        .expect("milestones")
        .retain(|value| value["kind"] != "ready");
    reject(run_without_ready);

    let mut stopped_without_cleanup = stop.clone();
    stopped_without_cleanup["cleanup"]["machine"] = serde_json::json!("incomplete");
    reject(stopped_without_cleanup);

    let mut stopped_with_wrong_method = valid_stop_receipt();
    stopped_with_wrong_method["cleanup"]["method"] = serde_json::json!("forced");
    reject(stopped_with_wrong_method);

    let mut cleanup_finished_without_start = stop;
    cleanup_finished_without_start["milestones"]
        .as_array_mut()
        .expect("milestones")
        .retain(|value| value["kind"] != "cleanup_started");
    reject(cleanup_finished_without_start);

    let mut decreasing_time = run.clone();
    decreasing_time["milestones"][2]["elapsed_ns"] = serde_json::json!(1);
    reject(decreasing_time);

    let mut wrong_order = launch.clone();
    wrong_order["milestones"]
        .as_array_mut()
        .expect("milestones")
        .swap(2, 4);
    reject(wrong_order);

    let mut duplicate_accepted = launch.clone();
    let accepted = duplicate_accepted["milestones"][0].clone();
    duplicate_accepted["milestones"]
        .as_array_mut()
        .expect("milestones")
        .insert(1, accepted);
    reject(duplicate_accepted);

    let mut wrong_terminal = run.clone();
    wrong_terminal["terminal_status"] = serde_json::json!("stopped");
    reject(wrong_terminal);

    let mut arbitrary_measurement = run.clone();
    arbitrary_measurement["measurement"]["clock"] = serde_json::json!("wall_clock");
    reject(arbitrary_measurement);

    let mut oversized = run.clone();
    let milestone = oversized["milestones"][0].clone();
    while oversized["milestones"]
        .as_array()
        .expect("milestones")
        .len()
        <= 10
    {
        oversized["milestones"]
            .as_array_mut()
            .expect("milestones")
            .push(milestone.clone());
    }
    reject(oversized);

    let mut unknown_field = run.clone();
    unknown_field["provider_payload"] = serde_json::json!({"unsafe": true});
    reject(unknown_field);

    let mut invalid_binding = run;
    invalid_binding["digest_binding"]["value"] = serde_json::json!("assumed_from_request");
    reject(invalid_binding);
}

#[test]
fn mac_receipt_states_observed_only_digest_binding() {
    let receipt = valid_run_receipt();

    assert_eq!(
        receipt["digest_binding"],
        serde_json::json!({"state":"observed","value":"observed_only"})
    );
}

fn valid_run_receipt() -> serde_json::Value {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    serde_json::to_value(engine.run(run_request()).expect("run").receipt()).expect("receipt JSON")
}

fn valid_launch_receipt() -> serde_json::Value {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    serde_json::to_value(
        engine
            .launch_machine(launch_request())
            .expect("launch")
            .receipt(),
    )
    .expect("receipt JSON")
}

fn valid_stop_receipt() -> serde_json::Value {
    let (backend, _) = TestBackend::new(Mode::Happy);
    let mut engine = Engine::new(backend);
    let launch = launch_request();
    let instance = instance();
    engine.launch_machine(launch).expect("launch");
    serde_json::to_value(
        engine
            .stop_machine(StopMachineRequest::new(operation('5'), instance))
            .expect("stop")
            .receipt(),
    )
    .expect("receipt JSON")
}

fn launch_request() -> LaunchMachineRequest {
    LaunchMachineRequest::new(
        operation('1'),
        instance(),
        OciImage::parse("node:22").expect("image"),
        MachineShape::new(1, 1_024, 8_192).expect("shape"),
    )
}

fn instance() -> InstanceId {
    InstanceId::new("22222222222222222222222222222222").expect("instance")
}

fn operation(digit: char) -> OperationId {
    OperationId::new(digit.to_string().repeat(32)).expect("operation")
}

fn reject(value: serde_json::Value) {
    assert!(serde_json::from_value::<ExecutionReceipt>(value).is_err());
}
