//! The `Server-Timing` header of a real create over the wire: the answer names
//! every phase of the create, not only the three segments it used to carry.

use std::sync::Arc;

use crate::{
    clients::create_when_ready,
    control_plane::start_control_plane,
    facade::{Engine, opener},
    support::{config, eventually, scratch, snapshot, start_runner},
};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_create_answer_names_every_phase_of_the_create() {
    let (control_plane, state) = start_control_plane().await;
    let engine = Arc::new(Engine::default());
    let journal = scratch("timing");
    let handle = start_runner(config(control_plane, &journal, 900), opener(&engine)).await;
    eventually("the feed connection", || !state.connections().is_empty()).await;
    snapshot(&state, 1);

    let created = create_when_ready(handle.address, "").await;
    assert_eq!(created.status, 201, "{}", created.text);

    let timing = created.headers["server-timing"].to_str().expect("header");

    // The three segments the header has always carried keep their names and their
    // order, so a client that parses `exec` today is unaffected.
    assert!(
        timing.starts_with("auth;dur=")
            && timing.contains(",pool;dur=")
            && timing.contains(",exec;dur="),
        "{timing}"
    );

    // The runner's own work on either side of the facade call.
    for name in ["prep", "finish"] {
        assert!(
            timing.contains(&format!("{name};dur=")),
            "{name} in {timing}"
        );
    }

    // The launch phases, derived from the receipt's own milestones. The fake facade
    // hands back `tests/fixtures/receipt.json`, whose milestones are fixed, so the
    // derived segments can be asserted exactly: machine_launched at 305_978_116 ns
    // and ready at 646_002_667 ns place assign at 305.973 ms and ready at 340.024 ms.
    assert!(timing.contains("assign;dur=305.973"), "{timing}");
    assert!(timing.contains("ready;dur=340.024"), "{timing}");
    // `commit` is the facade call minus the receipt's `ready`, and the fake launch
    // returns in microseconds, so the subtraction saturates at zero rather than
    // reporting a segment longer than the call it belongs to.
    assert!(timing.contains("commit;dur=0.000"), "{timing}");
}
