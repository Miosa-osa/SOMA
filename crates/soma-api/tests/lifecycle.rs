#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, call, identified};

#[test]
fn creating_a_sandbox_answers_created_with_the_launch_receipt() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        "/v1/sandboxes",
        r#"{"image":"node:22","shape":{"vcpu_count":1,"memory_mib":1024,"storage_mib":10240,"capabilities":{"network":{"profile":{"mode":"disabled"},"guest_addresses":{"ipv4":{"mode":"disabled"},"ipv6":{"mode":"disabled"}},"proxy":{"mode":"disabled"},"egress":"denied","dns":{"mode":"denied"},"published_ports":[]}}}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert_eq!(facade.calls, vec![Call::Launch]);
    assert_eq!(body["schema"], "soma.api.v1");
    assert_eq!(body["operation"], "sandbox.create");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["result"]["instance_id"], FIXTURE_INSTANCE_ID);
    assert_eq!(body["result"]["state"], "ready");
    assert_eq!(body["receipt"]["instance_id"], FIXTURE_INSTANCE_ID);
}

#[test]
fn creating_a_sandbox_without_a_shape_uses_the_facade_defaults() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert_eq!(body["status"], "ok");
}

#[test]
fn creating_a_sandbox_with_an_unparseable_image_never_reaches_the_engine() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"https://node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn creating_a_sandbox_with_an_unknown_field_is_refused_rather_than_ignored() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        "/v1/sandboxes",
        r#"{"image":"node:22","metadata":{"team":"platform"}}"#,
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 400);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "invalid_input");
}

#[test]
fn getting_a_sandbox_reports_its_observed_state_and_backend() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Inspect]);
    assert_eq!(body["operation"], "sandbox.get");
    assert_eq!(body["result"]["state"], "ready");
    assert_eq!(body["result"]["backend"], "linux_kvm");
}

#[test]
fn getting_an_absent_sandbox_reports_the_facade_not_found_code() {
    let mut facade = FakeFacade::new(Mode::NotFound);
    let request = identified("GET", &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"), "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert_eq!(body["status"], "error");
    assert_eq!(body["error"]["code"], "machine_not_found");
    assert_eq!(body["error"]["retryable"], false);
}

#[test]
fn a_path_segment_that_cannot_be_an_instance_id_is_reported_as_absent() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("GET", "/v1/sandboxes/not-an-instance", "");

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 404);
    assert!(facade.calls.is_empty());
    assert_eq!(body["error"]["code"], "machine_not_found");
}

#[test]
fn stopping_a_sandbox_reports_the_stopped_state() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/stop"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Stop]);
    assert_eq!(body["operation"], "sandbox.stop");
    assert_eq!(body["result"]["state"], "stopped");
}

#[test]
fn destroying_a_sandbox_reports_the_destroyed_state() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "DELETE",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(facade.calls, vec![Call::Destroy]);
    assert_eq!(body["operation"], "sandbox.destroy");
    assert_eq!(body["result"]["state"], "destroyed");
}

#[test]
fn a_state_conflict_answers_conflict_with_the_facade_conflict_code() {
    let mut facade = FakeFacade::new(Mode::Conflict);
    let request = identified(
        "DELETE",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}"),
        "",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "state_conflict");
}

#[test]
fn a_start_that_names_its_volume_binds_that_file_and_no_other() {
    let volume = "5".repeat(32);
    let mut facade = FakeFacade::new(Mode::Succeed)
        .with_volume_binding(&volume, "/srv/soma/volumes/05/volume.img");
    // The Instance identity is named so the host is asked about the same one the launch is for.
    let request = identified(
        "POST",
        "/v1/sandboxes",
        &format!(
            r#"{{"image":"node:22","instance_id":"{FIXTURE_INSTANCE_ID}","volume_id":"{volume}"}}"#
        ),
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert_eq!(body["status"], "ok");
    // The host is asked about this Instance, and the launch carries the file it named.
    assert_eq!(facade.volume_calls, vec![FIXTURE_INSTANCE_ID.to_owned()]);
    let attachment = facade.launched_volume.expect("the launch binds a volume");
    assert_eq!(
        attachment.path,
        std::path::PathBuf::from("/srv/soma/volumes/05/volume.img")
    );
    assert_eq!(
        attachment.id, [0x55_u8; 16],
        "the id the ready signal reports is the host's, decoded from hexadecimal"
    );
    // The start answer carries the volume and this start's repair: the control plane persists
    // the id and reads it back on the next reattach.
    assert_eq!(
        body["result"]["volume"]["volume_id"],
        volume.to_ascii_lowercase()
    );
    assert_eq!(body["result"]["volume"]["state"], "attached");
    assert_eq!(body["result"]["volume"]["device"], "vdc");
    assert_eq!(body["result"]["fsck"]["ran"], true);
    assert_eq!(body["result"]["fsck"]["tool"], "e2fsck");
}

#[test]
fn a_start_that_names_no_volume_binds_none_and_asks_the_host_nothing() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified("POST", "/v1/sandboxes", r#"{"image":"node:22"}"#);

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 201);
    assert!(
        facade.volume_calls.is_empty(),
        "a launch that names no volume must not ask the host for one"
    );
    assert!(facade.launched_volume.is_none());
    // A sandbox with no durable volume says so by carrying no volume and a null repair, rather
    // than by omitting the member.
    assert!(body["result"]["volume"].is_null());
    assert!(body["result"]["fsck"].is_null());
}

#[test]
fn a_start_that_names_a_volume_the_host_does_not_have_is_refused_rather_than_guessed() {
    // The host has a different volume attached to this Instance, which is the two sides
    // disagreeing about ownership; guessing either way writes the wrong disk.
    let mut facade = FakeFacade::new(Mode::Succeed)
        .with_volume_binding(&"6".repeat(32), "/srv/soma/volumes/06/volume.img");
    let request = identified(
        "POST",
        "/v1/sandboxes",
        &format!(
            r#"{{"image":"node:22","instance_id":"{FIXTURE_INSTANCE_ID}","volume_id":"{}"}}"#,
            "5".repeat(32)
        ),
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "volume_instance_mismatch");
    assert!(
        facade.launched_volume.is_none(),
        "a refused launch must not reach the machine"
    );
}

#[test]
fn stopping_a_sandbox_with_a_volume_releases_it_and_says_so() {
    let mut facade = FakeFacade::new(Mode::Succeed).with_owned_volume(&"5".repeat(32), "attached");
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/stop"),
        "{}",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["result"]["state"], "stopped");
    // A stop is a clean detach followed by destroying the machine, and the answer reports the
    // state the disk is left in rather than only that the sandbox stopped.
    assert_eq!(facade.volume_operations, vec!["volume.detach"]);
    assert_eq!(
        body["result"]["volume"]["volume_id"],
        "5".repeat(32),
        "the stop reports the volume it released"
    );
    assert_eq!(body["result"]["volume"]["state"], "detached");
    assert_eq!(body["result"]["volume"]["device"], "vdc");
    assert!(body["result"]["fsck"].is_null());
}

#[test]
fn stopping_a_sandbox_with_no_volume_asks_the_host_for_nothing() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let request = identified(
        "POST",
        &format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}/stop"),
        "{}",
    );

    let (status, body) = call(&mut facade, &request);

    assert_eq!(status, 200);
    assert_eq!(body["result"]["state"], "stopped");
    assert!(facade.volume_operations.is_empty());
    assert!(body["result"]["volume"].is_null());
}
