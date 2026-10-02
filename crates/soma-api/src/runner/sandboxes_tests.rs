use std::{
    sync::{Arc, atomic::AtomicI64},
    time::{Duration, Instant},
};

use super::{Owner, Sandboxes, TOMBSTONE_RETENTION, Unavailable};
use crate::runner::{
    ids::SandboxId,
    principal::{Tenant, TenantPolicy},
};

fn owner(tenant: &str, created: Instant, slot: &Arc<AtomicI64>) -> Owner {
    Owner {
        tenant_id: tenant.to_owned(),
        key_id: Some("k-1".to_owned()),
        project_id: None,
        created,
        slot: Arc::clone(slot),
    }
}

fn ready(sandboxes: &Sandboxes, tenant: &str, now: Instant) -> SandboxId {
    ready_counted(sandboxes, tenant, now, &Arc::new(AtomicI64::new(1)))
}

fn ready_counted(
    sandboxes: &Sandboxes,
    tenant: &str,
    now: Instant,
    slot: &Arc<AtomicI64>,
) -> SandboxId {
    let id = SandboxId::mint('3');
    sandboxes.reserve(
        id.clone(),
        owner(tenant, now, slot),
        Duration::from_secs(60),
    );
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
    assert_eq!(sandboxes.owner_of(&id, "t-2"), Err(Unavailable::NotFound));
    assert!(sandboxes.owner_of(&id, "t-1").is_ok());
    assert_eq!(sandboxes.owned_by("t-1"), vec![id.clone()]);
    assert!(sandboxes.owned_by("t-2").is_empty());
    assert!(sandboxes.begin_command(&id, "t-1").is_ok());
}

#[test]
fn one_lifecycle_call_runs_at_a_time() {
    let sandboxes = Sandboxes::new();
    let id = ready(&sandboxes, "t-1", Instant::now());

    sandboxes.begin_command(&id, "t-1").expect("first command");
    assert_eq!(sandboxes.begin_command(&id, "t-1"), Err(Unavailable::Busy));
    assert_eq!(sandboxes.begin_destroy(&id, "t-1"), Err(Unavailable::Busy));
    assert!(
        sandboxes.owner_of(&id, "t-1").is_ok(),
        "file and terminal calls do not wait"
    );
    sandboxes.release(&id);
    assert!(sandboxes.begin_destroy(&id, "t-1").is_ok());
}

#[test]
fn a_creating_sandbox_is_not_yet_addressable() {
    let sandboxes = Sandboxes::new();
    let slot = Arc::new(AtomicI64::new(1));
    let id = SandboxId::mint('3');
    sandboxes.reserve(
        id.clone(),
        owner("t-1", Instant::now(), &slot),
        Duration::from_secs(60),
    );

    assert_eq!(
        sandboxes.begin_command(&id, "t-1"),
        Err(Unavailable::NotFound)
    );
    sandboxes.abandon(&id);
    assert_eq!(sandboxes.live(), 0);
    assert_eq!(slot.load(std::sync::atomic::Ordering::SeqCst), 0);
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
fn the_tenant_share_is_a_counter_given_back_once_per_sandbox() {
    let sandboxes = Sandboxes::new();
    let now = Instant::now();
    let policy = TenantPolicy {
        soma: true,
        suspended: false,
        max_concurrent_share: Some(1),
        default_timeout_seconds: None,
    };
    let tenant = Tenant::new(policy, Arc::new(AtomicI64::new(0)));

    assert!(tenant.admit());
    let first = ready_counted(&sandboxes, "t-1", now, &tenant.counter());
    assert!(!tenant.admit(), "the share is full");
    assert_eq!(tenant.live(), 1);

    sandboxes.destroyed(&first, now);
    sandboxes.destroyed(&first, now);
    assert_eq!(tenant.live(), 0, "a second destroy gives nothing back");
    assert!(tenant.admit());
}

#[test]
fn expired_or_suspended_ready_sandboxes_are_claimed_once() {
    let sandboxes = Sandboxes::new();
    let now = Instant::now();
    let id = ready(&sandboxes, "t-1", now);
    let suspended = ready(&sandboxes, "t-2", now);

    assert!(sandboxes.claim_expired(now, |_| false).is_empty());
    let reaped = sandboxes.claim_expired(now, |tenant| tenant == "t-2");
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0].0, suspended);
    let expired = sandboxes.claim_expired(now + Duration::from_secs(60), |_| false);
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].0, id);
    assert!(
        sandboxes
            .claim_expired(now + Duration::from_secs(61), |_| true)
            .is_empty()
    );
}
