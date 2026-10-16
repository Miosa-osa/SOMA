use crate::{Machine, control::OutputStream};

use super::{
    fixtures::{execute, launch},
    platform::DeterministicPlatform,
};

fn ready_machine() -> Machine {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    machine.launch(launch()).expect("launch is ready");
    machine
}

#[test]
fn a_platform_that_cannot_stream_replays_the_retained_output_to_the_sink() {
    let mut machine = ready_machine();
    let mut chunks = Vec::new();
    let executed = machine
        .execute_streaming(execute([0x31; 16]), &mut |stream, bytes| {
            chunks.push((stream, bytes.to_vec()));
        })
        .expect("executes");
    assert_eq!(executed.stdout(), b"execution-1");
    assert_eq!(
        chunks,
        vec![(OutputStream::Stdout, b"execution-1".to_vec())]
    );
}

#[test]
fn streaming_and_buffered_execution_return_the_same_receipt() {
    let mut machine = ready_machine();
    let buffered = machine.execute(execute([0x32; 16])).expect("buffered");
    let mut other = ready_machine();
    let streamed = other
        .execute_streaming(execute([0x32; 16]), &mut |_, _| {})
        .expect("streamed");
    assert_eq!(buffered.stdout(), streamed.stdout());
    assert_eq!(buffered.stderr(), streamed.stderr());
    assert_eq!(buffered.status(), streamed.status());
}

#[test]
fn a_replayed_operation_runs_nothing_and_reports_no_chunks() {
    let mut machine = ready_machine();
    let first = machine
        .execute_streaming(execute([0x33; 16]), &mut |_, _| {})
        .expect("first");
    let mut chunks = 0;
    let again = machine
        .execute_streaming(execute([0x33; 16]), &mut |_, _| chunks += 1)
        .expect("replays");
    assert_eq!(chunks, 0);
    assert_eq!(first.stdout(), again.stdout());
}

#[test]
fn a_rejected_streaming_request_reports_no_chunks() {
    let mut machine = Machine::with_platform(DeterministicPlatform::healthy());
    let mut chunks = 0;
    let failure = machine
        .execute_streaming(execute([0x34; 16]), &mut |_, _| chunks += 1)
        .expect_err("not launched");
    assert_eq!(failure.kind(), crate::FailureKind::InvalidLifecycle);
    assert_eq!(chunks, 0);
}
