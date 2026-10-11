//! `GET` and `PATCH /v1/sandboxes/{id}/resources`: the report for each dimension, the status a
//! change that applied nothing gets, and what an old Generation says.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use soma_api::ResizeRequest;
use soma_local::{CpuResize, DimensionState, MemoryResize, ResizeOutcome};
use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

fn outcome(cpu: DimensionState, memory: DimensionState) -> ResizeOutcome {
    ResizeOutcome {
        cpu: CpuResize {
            state: cpu,
            requested: Some(2),
            applied: if cpu == DimensionState::AppliedLive {
                2
            } else {
                4
            },
            booted: 4,
            reason: (cpu == DimensionState::Unsupported)
                .then(|| "the machine has no cpu leaf".to_owned()),
        },
        memory: MemoryResize {
            state: memory,
            requested_mb: Some(3_072),
            plugged_mb: if memory == DimensionState::AppliedLive {
                2_048
            } else {
                0
            },
            total_mb: if memory == DimensionState::AppliedLive {
                4_096
            } else {
                2_048
            },
            booted_mb: 2_048,
            ceiling_mb: 8_192,
            reason: (memory == DimensionState::Unsupported)
                .then(|| "this Generation's kernel has no virtio-mem".to_owned()),
        },
    }
}

fn path() -> String {
    format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/resources")
}

#[test]
fn a_change_that_applied_reports_each_dimension_as_the_device_confirmed_it() {
    let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::AppliedLive,
        DimensionState::Partial,
    ));
    let (status, body) = call(
        &mut facade,
        &identified("PATCH", &path(), r#"{"cpu_count":2,"memory_mb":3072}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["operation"], "sandbox.resize");
    assert_eq!(
        facade.calls,
        vec![Call::Resize(ResizeRequest {
            cpu_count: Some(2),
            memory_mb: Some(3_072)
        })]
    );
    let result = &body["result"];
    assert_eq!(result["instance_id"], FIXTURE_INSTANCE_ID);
    assert_eq!(result["applied_live"], true);
    assert_eq!(result["cpu"]["state"], "applied_live");
    assert_eq!(result["cpu"]["mechanism"], "cgroup_quota");
    assert_eq!(result["cpu"]["requested"], 2);
    assert_eq!(result["cpu"]["applied"], 2);
    assert_eq!(result["cpu"]["booted"], 4);
    assert_eq!(result["memory"]["state"], "partial");
    assert_eq!(result["memory"]["mechanism"], "virtio_mem");
    assert_eq!(result["memory"]["requested_mb"], 3_072);
    assert_eq!(result["memory"]["plugged_mb"], 0);
    assert_eq!(result["memory"]["total_mb"], 2_048);
    assert_eq!(result["memory"]["booted_mb"], 2_048);
    assert_eq!(result["memory"]["ceiling_mb"], 8_192);
}

#[test]
fn one_dimension_applying_is_enough_for_a_200_and_the_other_says_why_not() {
    let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::AppliedLive,
        DimensionState::Unsupported,
    ));
    let (status, body) = call(
        &mut facade,
        &identified("PATCH", &path(), r#"{"cpu_count":2,"memory_mb":3072}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["applied_live"], true);
    assert_eq!(body["result"]["memory"]["state"], "unsupported");
    assert_eq!(
        body["result"]["memory"]["reason"],
        "this Generation's kernel has no virtio-mem"
    );
}

#[test]
fn a_change_that_applied_nothing_is_a_409_that_still_carries_the_report() {
    let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::Unsupported,
        DimensionState::Unsupported,
    ));
    let (status, body) = call(
        &mut facade,
        &identified("PATCH", &path(), r#"{"cpu_count":2,"memory_mb":3072}"#),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "resize_not_applied");
    assert_eq!(body["result"]["applied_live"], false);
    assert_eq!(body["result"]["cpu"]["state"], "unsupported");
    assert_eq!(body["result"]["memory"]["state"], "unsupported");
    assert!(body["result"]["cpu"]["reason"].is_string());
}

#[test]
fn a_failed_attempt_is_also_a_409() {
    let mut facade = FakeFacade::new(Mode::Succeed)
        .resizing(outcome(DimensionState::Failed, DimensionState::Unchanged));
    let (status, body) = call(
        &mut facade,
        &identified("PATCH", &path(), r#"{"cpu_count":2}"#),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["result"]["cpu"]["state"], "failed");
}

#[test]
fn asking_for_what_the_machine_already_has_is_a_200_that_changed_nothing() {
    let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::Unchanged,
        DimensionState::Unchanged,
    ));
    let (status, body) = call(
        &mut facade,
        &identified("PATCH", &path(), r#"{"cpu_count":4}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["applied_live"], false);
}

#[test]
fn a_read_reports_the_current_state_even_on_an_old_generation() {
    let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::Unsupported,
        DimensionState::Unsupported,
    ));
    let (status, body) = call(&mut facade, &identified("GET", &path(), ""));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["operation"], "sandbox.resources");
    assert_eq!(facade.calls, vec![Call::Resize(ResizeRequest::default())]);
    assert_eq!(body["result"]["memory"]["state"], "unsupported");
    assert_eq!(body["result"]["applied_live"], false);
}

#[test]
fn bad_bodies_are_refused_before_the_facade_is_asked() {
    for body in [
        "{}",
        r#"{"cpu_count":0}"#,
        r#"{"memory_mb":0}"#,
        r#"{"cpu_count":"two"}"#,
        r#"{"cpu_count":70000}"#,
        r#"{"cpu":2}"#,
        "",
    ] {
        let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
            DimensionState::AppliedLive,
            DimensionState::AppliedLive,
        ));
        let (status, response) = call(&mut facade, &identified("PATCH", &path(), body));
        assert_eq!(status, 400, "{body:?}: {response}");
        assert!(facade.calls.is_empty(), "{body:?}");
    }
}

#[test]
fn an_unreachable_or_unknown_sandbox_has_its_own_status() {
    let mut facade = FakeFacade::new(Mode::NotFound).resizing(outcome(
        DimensionState::AppliedLive,
        DimensionState::AppliedLive,
    ));
    let (status, body) = call(&mut facade, &identified("GET", &path(), ""));
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"]["code"], "machine_not_found");

    let mut backend_without_resize = FakeFacade::new(Mode::Succeed);
    let (status, body) = call(&mut backend_without_resize, &identified("GET", &path(), ""));
    assert_eq!(status, 503, "{body}");
    assert_eq!(body["error"]["code"], "resize_unavailable");
}

#[test]
fn only_get_and_patch_are_accepted() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    for method in ["POST", "PUT", "DELETE"] {
        let (status, _) = call(&mut facade, &identified(method, &path(), "{}"));
        assert_eq!(status, 405, "{method}");
    }
    assert!(facade.calls.is_empty());
}

#[test]
fn an_operation_id_is_echoed_and_a_replay_is_answered_without_applying_again() {
    // One test owns the replay table's entry for this instance: the table keeps the latest
    // attempt per sandbox, so interleaved tests naming operation ids would overwrite each other.
    let first = "0123456789abcdef0123456789abcdef";
    let second = "fedcba9876543210fedcba9876543210";
    let patch = |id: &str| {
        identified(
            "PATCH",
            &path(),
            &format!(r#"{{"cpu_count":2,"operation_id":"{id}"}}"#),
        )
    };
    let applied = outcome(DimensionState::AppliedLive, DimensionState::Unchanged);

    let mut facade = FakeFacade::new(Mode::Succeed).resizing(applied.clone());
    let (status, body) = call(&mut facade, &patch(first));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["operation_id"], first);
    assert_eq!(facade.calls.len(), 1);

    // The same attempt again, even on a facade that would now answer differently.
    let mut replay = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::Unchanged,
        DimensionState::Unchanged,
    ));
    let (again_status, again) = call(&mut replay, &patch(first));
    assert_eq!((again_status, &again), (status, &body));
    assert!(
        replay.calls.is_empty(),
        "a replay must not reach the facade"
    );

    // A new attempt is applied and becomes the one remembered.
    let (status, body) = call(&mut replay, &patch(second));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["result"]["operation_id"], second);
    assert_eq!(replay.calls.len(), 1);

    // A refusal is remembered too, so its reason is what a replay reads.
    let third = "00112233445566778899aabbccddeeff";
    let mut refusing = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::Unsupported,
        DimensionState::Unsupported,
    ));
    let (status, refused) = call(&mut refusing, &patch(third));
    assert_eq!(status, 409, "{refused}");
    let mut after = FakeFacade::new(Mode::Succeed).resizing(outcome(
        DimensionState::AppliedLive,
        DimensionState::AppliedLive,
    ));
    assert_eq!(call(&mut after, &patch(third)), (409, refused));
    assert!(after.calls.is_empty());
}

#[test]
fn a_malformed_operation_id_is_refused_before_the_facade_is_asked() {
    for id in [
        "",
        "short",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
    ] {
        let mut facade = FakeFacade::new(Mode::Succeed).resizing(outcome(
            DimensionState::AppliedLive,
            DimensionState::AppliedLive,
        ));
        let body = format!(r#"{{"cpu_count":2,"operation_id":"{id}"}}"#);
        let (status, response) = call(&mut facade, &identified("PATCH", &path(), &body));
        assert_eq!(status, 400, "{id:?}: {response}");
        assert!(facade.calls.is_empty());
    }
}
