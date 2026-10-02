use std::time::{Duration, Instant};

use super::{Owner, Sandboxes, TOMBSTONE_RETENTION, Unavailable};
use crate::runner::ids::SandboxId;

fn owner(tenant: &str, created: Instant) -> Owner {
    Owner {
        tenant_id: tenant.to_owned(),
        key_id: Some("k-1".to_owned()),
        project_id: None,
        created,
    }
}

fn ready(sandboxes: &Sandboxes, tenant: &str, now: Instant) -> SandboxId {
    let id = SandboxId::mint('3');
    assert!(sandboxes.reserve(
        id.clone(),
        owner(tenant, now),
        Duration::from_secs(60),
        None
    ));
    sandboxes.confirm(&id);
    id
}

#[test]
fn only_the_owning_tenant_reaches_a_sandbox() {
    let sandboxes = Sandboxes::new();
    let id = ready(&sandboxes, "t-1", Instant::now());

    assert_eq!(
        sandboxes.begin_command(&id, "t-2"),
        Err(Unavailable::NotFound)
    );
    assert!(sandboxes.begin_command(&id, "t-1").is_ok());
}

#[test]
fn one_lifecycle_call_runs_at_a_time() {
    let sandboxes = Sandboxes::new();
    let id = ready(&sandboxes, "t-1", Instant::now());

    sandboxes.begin_command(&id, "t-1").expect("first command");
    assert_eq!(sandboxes.begin_command(&id, "t-1"), Err(Unavailable::Busy));
    assert_eq!(sandboxes.begin_destroy(&id, "t-1"), Err(Unavailable::Busy));
    sandboxes.release(&id);
    assert!(sandboxes.begin_destroy(&id, "t-1").is_ok());
}

#[test]
fn a_creating_sandbox_is_not_yet_addressable() {
    let sandboxes = Sandboxes::new();
    let id = SandboxId::mint('3');
    sandboxes.reserve(
        id.clone(),
        owner("t-1", Instant::now()),
        Duration::from_secs(60),
        None,
    );

    assert_eq!(
        sandboxes.begin_command(&id, "t-1"),
        Err(Unavailable::NotFound)
    );
    sandboxes.abandon(&id);
    assert_eq!(sandboxes.live(), 0);
}

#[test]
fn a_destroyed_sandbox_is_remembered_then_forgotten() {
    let sandboxes = Sandboxes::new();
    let now = Instant::now();
    let id = ready(&sandboxes, "t-1", now);
    sandboxes.begin_destroy(&id, "t-1").expect("destroy starts");
    sandboxes.destroyed(&id, now);

    assert!(matches!(
        sandboxes.begin_destroy(&id, "t-1"),
        Err(Unavailable::Destroyed(_))
    ));
    assert_eq!(sandboxes.live(), 0);
    sandboxes.sweep(now + TOMBSTONE_RETENTION);
    assert_eq!(
        sandboxes.begin_destroy(&id, "t-1"),
        Err(Unavailable::NotFound)
    );
}

#[test]
fn the_tenant_cap_counts_live_sandboxes_only() {
    let sandboxes = Sandboxes::new();
    let now = Instant::now();
    let first = ready(&sandboxes, "t-1", now);

    assert!(!sandboxes.reserve(
        SandboxId::mint('3'),
        owner("t-1", now),
        Duration::from_secs(60),
        Some(1)
    ));
    assert!(sandboxes.reserve(
        SandboxId::mint('3'),
        owner("t-2", now),
        Duration::from_secs(60),
        Some(1)
    ));
    sandboxes.destroyed(&first, now);
    assert!(sandboxes.reserve(
        SandboxId::mint('3'),
        owner("t-1", now),
        Duration::from_secs(60),
        Some(1)
    ));
}

#[test]
fn expired_ready_sandboxes_are_claimed_once() {
    let sandboxes = Sandboxes::new();
    let now = Instant::now();
    let id = ready(&sandboxes, "t-1", now);

    assert!(sandboxes.claim_expired(now).is_empty());
    let expired = sandboxes.claim_expired(now + Duration::from_secs(60));
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].0, id);
    assert!(
        sandboxes
            .claim_expired(now + Duration::from_secs(61))
            .is_empty()
    );
}
