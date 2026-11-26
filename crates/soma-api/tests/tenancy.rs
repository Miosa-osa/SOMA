//! A sandbox or a volume is answered only for the tenant that created it (SOMA#222, SOMA#219).
//!
//! Every other tenant reads it as absent, and nothing it asks for reaches the engine or the host.

#[allow(
    dead_code,
    reason = "each integration test uses a subset of the shared fixtures"
)]
mod support;

use soma::{BackendKind, InstanceId, SandboxEntry, SandboxLiveness, SandboxPhase};
use support::{Call, FIXTURE_INSTANCE_ID, FakeFacade, Mode, as_tenant, call};

const OWNER: &str = "wl-desktop-proof";
const PLATFORM: &str = "soma-platform";
const OTHER: &str = "wl-edge-other";
const VOLUME: &str = "759e3025aa11bb22cc33dd44ee55ff66";

fn created() -> FakeFacade {
    let entry = SandboxEntry::new(
        InstanceId::new(FIXTURE_INSTANCE_ID).expect("fixture identity is canonical"),
        SandboxPhase::Active,
        BackendKind::LinuxKvm,
        None,
        SandboxLiveness::Live,
    );
    let mut facade = FakeFacade::new(Mode::Succeed).holding(vec![entry]);
    let create = format!(r#"{{"image":"node:22","instance_id":"{FIXTURE_INSTANCE_ID}"}}"#);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/sandboxes", &create),
    );
    assert_eq!(status, 201, "the owner creates the sandbox");
    facade.calls.clear();
    facade
}

/// A record written before this service recorded owners: a sandbox no tenant claimed.
fn unowned() -> FakeFacade {
    let entry = SandboxEntry::new(
        InstanceId::new(FIXTURE_INSTANCE_ID).expect("fixture identity is canonical"),
        SandboxPhase::Launching,
        BackendKind::LinuxKvm,
        None,
        SandboxLiveness::Live,
    );
    FakeFacade::new(Mode::Succeed).holding(vec![entry])
}

fn volume_body(operation: &str, tenant: &str) -> String {
    format!(
        r#"{{"operation_id":"{operation}","instance_id":"{FIXTURE_INSTANCE_ID}","volume_id":"{VOLUME}","request_fingerprint":"{operation}{operation}","size_class_gib":10,"generation_id":"{operation}{operation}","tenant":"{tenant}"}}"#
    )
}

#[test]
fn another_tenant_reads_gets_and_deletes_nothing_of_a_sandbox_it_does_not_own() {
    let mut facade = created();
    let path = format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}");
    let command = r#"{"executable":"/bin/true"}"#;
    for (method, path, body) in [
        ("GET", path.clone(), ""),
        ("DELETE", path.clone(), ""),
        ("POST", format!("{path}/stop"), "{}"),
        ("POST", format!("{path}/commands"), command),
    ] {
        let (status, body) = call(&mut facade, &as_tenant(OTHER, method, &path, body));
        assert_eq!(status, 404, "{method} {path} from another tenant");
        assert_eq!(body["error"]["code"], "machine_not_found");
    }
    assert!(facade.calls.is_empty(), "nothing reached the engine");

    let (status, _) = call(&mut facade, &as_tenant(OWNER, "GET", &path, ""));
    assert_eq!(status, 200, "the owner still reads its sandbox");
    assert_eq!(facade.calls, [Call::Inspect]);
}

#[test]
fn a_listing_names_only_the_callers_own_sandboxes() {
    let mut facade = created();
    let (_, mine) = call(&mut facade, &as_tenant(OWNER, "GET", "/v1/sandboxes", ""));
    assert_eq!(mine["result"]["count"], 1);
    let (status, theirs) = call(&mut facade, &as_tenant(OTHER, "GET", "/v1/sandboxes", ""));
    assert_eq!(status, 200);
    assert_eq!(theirs["result"]["count"], 0);
    assert_eq!(theirs["result"]["sandboxes"], serde_json::json!([]));
}

#[test]
fn another_tenant_cannot_create_over_an_identity_that_is_taken() {
    let mut facade = created();
    let create = format!(r#"{{"image":"node:22","instance_id":"{FIXTURE_INSTANCE_ID}"}}"#);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OTHER, "POST", "/v1/sandboxes", &create),
    );
    assert_eq!(status, 409);
    assert!(facade.calls.is_empty(), "nothing reached the engine");
}

#[test]
fn another_tenant_cannot_destroy_detach_or_read_a_volume_it_does_not_own() {
    let mut facade = created();
    let attach = volume_body("11111111111111111111111111111111", OWNER);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/volumes/attach", &attach),
    );
    assert_eq!(status, 200, "the owner attaches its volume");
    facade.volume_operations.clear();

    for (method, path, body) in [
        (
            "POST",
            "/v1/volumes/destroy".to_owned(),
            volume_body("22222222222222222222222222222222", OTHER),
        ),
        (
            "POST",
            "/v1/volumes/detach".to_owned(),
            volume_body("33333333333333333333333333333333", OTHER),
        ),
        (
            "POST",
            "/v1/volumes/attach".to_owned(),
            volume_body("44444444444444444444444444444444", OTHER),
        ),
        ("GET", format!("/v1/volumes/{VOLUME}"), String::new()),
    ] {
        let (status, _) = call(&mut facade, &as_tenant(OTHER, method, &path, &body));
        assert_eq!(status, 404, "{method} {path} from another tenant");
    }
    assert!(
        facade.volume_operations.is_empty(),
        "nothing reached the host: {:?}",
        facade.volume_operations
    );

    let destroy = volume_body("55555555555555555555555555555555", OWNER);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/volumes/destroy", &destroy),
    );
    assert_eq!(status, 200, "the owner may still destroy its volume");
}

#[test]
fn a_volume_body_that_names_another_tenant_than_the_caller_is_refused() {
    let mut facade = created();
    let attach = volume_body("11111111111111111111111111111111", OTHER);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/volumes/attach", &attach),
    );
    assert_eq!(status, 400);
    assert!(facade.volume_operations.is_empty());
}

#[test]
fn a_sandbox_no_tenant_owns_is_answered_only_to_the_platform() {
    let mut facade = unowned();
    let path = format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}");
    for tenant in [OWNER, OTHER] {
        let (_, listed) = call(&mut facade, &as_tenant(tenant, "GET", "/v1/sandboxes", ""));
        assert_eq!(
            listed["result"]["count"], 0,
            "{tenant} lists no unowned record"
        );
        for (method, path) in [("GET", path.clone()), ("DELETE", path.clone())] {
            let (status, body) = call(&mut facade, &as_tenant(tenant, method, &path, ""));
            assert_eq!(status, 404, "{method} {path} from {tenant}");
            assert_eq!(body["error"]["code"], "machine_not_found");
        }
    }
    assert!(
        facade.calls.iter().all(|asked| *asked == Call::List),
        "no tenant reached the sandbox: {:?}",
        facade.calls
    );

    let (_, listed) = call(
        &mut facade,
        &as_tenant(PLATFORM, "GET", "/v1/sandboxes", ""),
    );
    assert_eq!(listed["result"]["count"], 1, "the platform lists it");
    let (status, _) = call(&mut facade, &as_tenant(PLATFORM, "GET", &path, ""));
    assert_eq!(status, 200, "the platform reads it");
    assert_eq!(facade.calls.last(), Some(&Call::Inspect));
}

#[test]
fn the_platform_reads_a_sandbox_a_tenant_owns() {
    let mut facade = created();
    let path = format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}");
    let (_, listed) = call(
        &mut facade,
        &as_tenant(PLATFORM, "GET", "/v1/sandboxes", ""),
    );
    assert_eq!(listed["result"]["count"], 1);
    let (status, _) = call(&mut facade, &as_tenant(PLATFORM, "GET", &path, ""));
    assert_eq!(status, 200);
}

#[test]
fn an_attach_before_the_create_takes_the_sandbox_identity_for_its_tenant() {
    let mut facade = FakeFacade::new(Mode::Succeed);
    let attach = volume_body("a1b2c3d4e5f60718293a4b5c6d7e8f90", OWNER);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/volumes/attach", &attach),
    );
    assert_ne!(
        status, 404,
        "a tenant attaches a volume for the sandbox it is about to create"
    );
    let create = format!(r#"{{"image":"node:22","instance_id":"{FIXTURE_INSTANCE_ID}"}}"#);
    let (status, _) = call(
        &mut facade,
        &as_tenant(OTHER, "POST", "/v1/sandboxes", &create),
    );
    assert_ne!(
        status, 201,
        "another tenant cannot create over the reserved identity"
    );
}

#[test]
fn a_tenant_cannot_take_an_unowned_sandbox_by_attaching_to_it() {
    let mut facade = unowned();
    let attach = volume_body("a1b2c3d4e5f60718293a4b5c6d7e8f90", OWNER);
    let (status, body) = call(
        &mut facade,
        &as_tenant(OWNER, "POST", "/v1/volumes/attach", &attach),
    );
    assert_eq!(status, 404, "{body}");
    let path = format!("/v1/sandboxes/{FIXTURE_INSTANCE_ID}");
    let (status, _) = call(&mut facade, &as_tenant(OWNER, "GET", &path, ""));
    assert_eq!(status, 404, "the record is still no tenant's");
}
