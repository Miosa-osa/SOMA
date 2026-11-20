//! The file contract driven against the in-memory guest: each platform call, its answer bytes,
//! and the guest operations it was made of.
//!
//! These cross the same seam the runner does, `Plan::of` then `Plan::run`, so what they prove is
//! what a request reaching a real sandbox gets; the HTTP route and the sandbox addressing around
//! it are proved through `Runner::handle` in the service's own tests.

mod memory;
mod moves;
mod reads;
mod writes;

pub(crate) use memory::MemoryGuest;

use serde_json::Value;

use super::{FileCall, Params, Plan, Stop, refusal};

/// One call against `guest`: the status and the decoded body, refusals included.
pub(crate) fn perform(
    guest: &mut MemoryGuest,
    call: FileCall,
    query: Option<&str>,
    body: &str,
) -> (u16, Value) {
    let (status, bytes) = perform_bytes(guest, call, query, body);
    (
        status,
        serde_json::from_slice(&bytes).expect("every answer is JSON"),
    )
}

/// The same, keeping the exact bytes of the answer.
pub(crate) fn perform_bytes(
    guest: &mut MemoryGuest,
    call: FileCall,
    query: Option<&str>,
    body: &str,
) -> (u16, Vec<u8>) {
    let params = Params::read(query, body.as_bytes()).expect("well-formed parameters");
    let plan = match Plan::of(call, &params) {
        Ok(plan) => plan,
        Err(error) => return (error.status, error.body()),
    };
    match plan.run(guest) {
        Ok(answer) => (answer.status, answer.body),
        Err(Stop::Refused(cause)) => {
            let error = refusal(call, cause);
            (error.status, error.body())
        }
        Err(other) => panic!("the guest was reachable and answered in kind: {other:?}"),
    }
}

/// The refusal code of a decoded answer.
pub(crate) fn code(body: &Value) -> &str {
    body["code"].as_str().expect("a refusal carries a code")
}

/// Whether a temporary sibling of any write is still in the guest.
pub(crate) fn leftover_temporaries(guest: &MemoryGuest) -> Vec<String> {
    guest
        .nodes
        .keys()
        .filter(|path| {
            path.rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with(".soma-write-"))
        })
        .cloned()
        .collect()
}

#[test]
fn every_call_refuses_a_path_outside_the_exposed_trees_before_the_guest_is_asked() {
    let calls = [
        (FileCall::List, Some("path=/etc"), ""),
        (FileCall::Read, Some("path=/etc/passwd"), ""),
        (
            FileCall::Write,
            None,
            r#"{"path":"/etc/passwd","content":"x"}"#,
        ),
        (FileCall::Stat, None, r#"{"path":"/proc/1/environ"}"#),
        (FileCall::MakeDirectory, None, r#"{"path":"/sys/x"}"#),
        (FileCall::Delete, Some("path=/dev/kvm"), ""),
        (
            FileCall::Rename,
            None,
            r#"{"from":"/tmp/a","to":"/root/a"}"#,
        ),
        (
            FileCall::Chmod,
            None,
            r#"{"path":"/tmp/../etc","mode":"0644"}"#,
        ),
    ];
    for (call, query, body) in calls {
        let mut guest = MemoryGuest::default();
        let (status, answer) = perform(&mut guest, call, query, body);
        assert_eq!((status, code(&answer)), (400, "INVALID_PATH"), "{call:?}");
        assert!(guest.log.is_empty(), "{call:?} reached the guest");
    }
}

#[test]
fn a_call_without_its_path_is_told_so() {
    for call in [
        FileCall::Read,
        FileCall::Write,
        FileCall::Stat,
        FileCall::Delete,
    ] {
        let mut guest = MemoryGuest::default();
        let body = if call == FileCall::Write {
            r#"{"content":"x"}"#
        } else {
            "{}"
        };
        let (status, answer) = perform(&mut guest, call, None, body);
        assert_eq!((status, code(&answer)), (400, "MISSING_PATH"), "{call:?}");
    }
}

#[test]
fn a_platform_route_this_runner_does_not_serve_is_refused_by_name() {
    let mut guest = MemoryGuest::default();
    let (status, body) = perform_bytes(&mut guest, FileCall::Unserved("copy"), None, "");
    assert_eq!(status, 501);
    assert_eq!(
        body,
        br#"{"code":"FILE_ROUTE_UNSUPPORTED","error":"the SOMA runner does not serve copy"}"#
    );
}

#[test]
fn an_unreachable_sandbox_stops_the_call_with_the_facade_failure() {
    let mut guest = MemoryGuest {
        unreachable: Some(soma::ManagedFailure::Backend(
            soma::BackendFailureKind::Unavailable,
        )),
        ..MemoryGuest::default()
    };
    let params = Params::read(Some("path=/tmp/a"), b"").expect("params");
    let plan = Plan::of(FileCall::Read, &params).expect("plan");
    assert_eq!(
        plan.run(&mut guest),
        Err(Stop::Failed(soma::ManagedFailure::Backend(
            soma::BackendFailureKind::Unavailable
        )))
    );
}
