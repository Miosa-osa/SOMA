use soma::{
    DirectCommand, ExecutionLimits, InstanceId, MachineShape, OciImage, OperationId, RunRequest,
};

#[allow(
    dead_code,
    reason = "shared fixture is used by run-focused integration tests"
)]
pub fn run_request() -> RunRequest {
    run_request_with_output_limit(16 * 1024 * 1024)
}

pub fn run_request_with_output_limit(max_output_bytes: u64) -> RunRequest {
    RunRequest::new(
        OperationId::new("11111111111111111111111111111111").expect("valid operation"),
        InstanceId::new("22222222222222222222222222222222").expect("valid instance"),
        OciImage::parse("node:22").expect("valid image"),
        MachineShape::new(1, 1_024, 8_192).expect("valid shape"),
        DirectCommand::new("/usr/local/bin/node", ["--version"]).expect("valid command"),
        ExecutionLimits::new(30_000, max_output_bytes).expect("valid limits"),
    )
}
