use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::{FeedViolation, KeyTable, Projects, Refusal};
use crate::runner::feed::FeedEvent;

pub(crate) const TOKEN: &str = "msk_us_live_token";
pub(crate) const TENANT: &str = "0b0c3a52-6a7e-4d39-9f1e-3c4d5e6f7a8b";

pub(crate) fn hash_of(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            use std::fmt::Write as _;
            let _written = write!(hex, "{byte:02x}");
            hex
        })
}

pub(crate) fn event(line: &str) -> FeedEvent {
    FeedEvent::parse(line).expect("a valid feed line")
}

/// A table holding `TOKEN` for an enabled `TENANT`, reached by a complete snapshot.
pub(crate) fn enabled_table() -> KeyTable {
    let table = KeyTable::new();
    let now = Instant::now();
    for line in [
        r#"{"seq":1,"kind":"snapshot_begin"}"#.to_owned(),
        format!(
            r#"{{"seq":2,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#,
            hash_of(TOKEN)
        ),
        format!(
            r#"{{"seq":3,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":null}}"#
        ),
        r#"{"seq":4,"kind":"snapshot_end"}"#.to_owned(),
    ] {
        table.apply(&event(&line), now).expect("applies");
    }
    table
}

#[test]
fn a_completed_snapshot_admits_its_keys() {
    let table = enabled_table();

    let principal = table
        .admit(Some(&format!("Bearer {TOKEN}")))
        .expect("admitted");
    assert_eq!(principal.key.key_id, "k-1");
    assert_eq!(principal.key.tenant_id, TENANT);
    assert_eq!(principal.key.projects, Projects::All);
    assert_eq!(table.last_seq(), 4);
}

#[test]
fn keys_inside_an_unfinished_snapshot_are_not_served() {
    let table = KeyTable::new();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":1,"kind":"snapshot_begin"}"#), now)
        .expect("applies");
    table
        .apply(
            &event(&format!(
                r#"{{"seq":2,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#,
                hash_of(TOKEN)
            )),
            now,
        )
        .expect("applies");

    assert_eq!(
        table.admit(Some(&format!("Bearer {TOKEN}"))).err(),
        Some(Refusal::Unauthorized)
    );
}

#[test]
fn a_new_snapshot_drops_keys_it_no_longer_lists() {
    let table = enabled_table();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":10,"kind":"snapshot_begin"}"#), now)
        .expect("applies");
    table
        .apply(&event(r#"{"seq":11,"kind":"snapshot_end"}"#), now)
        .expect("applies");

    assert_eq!(table.key_count(), 0);
    assert_eq!(
        table.admit(Some(&format!("Bearer {TOKEN}"))).err(),
        Some(Refusal::Unauthorized)
    );
}

#[test]
fn a_revoke_applies_at_once_even_mid_snapshot() {
    let table = enabled_table();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":5,"kind":"snapshot_begin"}"#), now)
        .expect("applies");
    table
        .apply(
            &event(&format!(
                r#"{{"seq":6,"kind":"key_revoke","key_hash":"{}"}}"#,
                hash_of(TOKEN)
            )),
            now,
        )
        .expect("applies");

    assert_eq!(
        table.admit(Some(&format!("Bearer {TOKEN}"))).err(),
        Some(Refusal::Unauthorized)
    );
}

#[test]
fn refuses_tokens_that_are_not_bearer_msk_keys() {
    let table = enabled_table();

    for header in [
        None,
        Some("msk_us_live_token"),
        Some("Bearer eyJhbGciOi"),
        Some("Bearer msk_us_other"),
    ] {
        assert_eq!(table.admit(header).err(), Some(Refusal::Unauthorized));
    }
}

#[test]
fn a_disabled_suspended_or_unknown_tenant_is_forbidden() {
    for policy in [
        r#""soma":false,"suspended":false"#,
        r#""soma":true,"suspended":true"#,
    ] {
        let table = enabled_table();
        table
            .apply(
                &event(&format!(
                    r#"{{"seq":20,"kind":"tenant_policy","tenant_id":"{TENANT}",{policy},"max_concurrent_share":null}}"#
                )),
                Instant::now(),
            )
            .expect("applies");
        assert_eq!(
            table.admit(Some(&format!("Bearer {TOKEN}"))).err(),
            Some(Refusal::Forbidden)
        );
    }

    let table = KeyTable::new();
    table
        .apply(
            &event(&format!(
                r#"{{"seq":1,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#,
                hash_of(TOKEN)
            )),
            Instant::now(),
        )
        .expect("applies");
    assert_eq!(
        table.admit(Some(&format!("Bearer {TOKEN}"))).err(),
        Some(Refusal::Forbidden)
    );
}

#[test]
fn project_scope_is_enforced_only_for_a_named_project() {
    let table = KeyTable::new();
    table
        .apply(
            &event(&format!(
                r#"{{"seq":1,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":"u-1","projects":["p-1"],"rate_per_s":5}}"#,
                hash_of(TOKEN)
            )),
            Instant::now(),
        )
        .expect("applies");
    table
        .apply(
            &event(&format!(
                r#"{{"seq":2,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":3}}"#
            )),
            Instant::now(),
        )
        .expect("applies");
    let principal = table
        .admit(Some(&format!("Bearer {TOKEN}")))
        .expect("admitted");

    assert!(principal.key.projects.allows(Some("p-1")));
    assert!(!principal.key.projects.allows(Some("p-2")));
    assert!(principal.key.projects.allows(None));
    assert_eq!(principal.key.rate_per_second, Some(5));
    assert_eq!(principal.tenant.policy.max_concurrent_share, Some(3));
}

#[test]
fn sequence_numbers_must_increase() {
    let table = enabled_table();

    assert_eq!(
        table.apply(&event(r#"{"seq":4,"kind":"heartbeat"}"#), Instant::now()),
        Err(FeedViolation::SequenceRegressed {
            last: 4,
            received: 4
        })
    );
}

#[test]
fn a_snapshot_opening_a_connection_may_restart_the_sequence() {
    let table = enabled_table();
    table.begin_connection();
    let now = Instant::now();
    table
        .apply(&event(r#"{"seq":1,"kind":"snapshot_begin"}"#), now)
        .expect("a restarted sequence opens with a snapshot");
    table
        .apply(&event(r#"{"seq":2,"kind":"snapshot_end"}"#), now)
        .expect("applies");

    assert_eq!(table.last_seq(), 2);
    table.begin_connection();
    assert!(
        table
            .apply(&event(r#"{"seq":1,"kind":"heartbeat"}"#), now)
            .is_err(),
        "only a snapshot may restart the sequence"
    );
}

#[test]
fn rejects_a_malformed_hash_and_an_unopened_snapshot_end() {
    let table = KeyTable::new();

    assert_eq!(
        table.apply(
            &event(r#"{"seq":1,"kind":"key_revoke","key_hash":"ABC"}"#),
            Instant::now()
        ),
        Err(FeedViolation::MalformedKeyHash)
    );
    assert_eq!(
        table.apply(&event(r#"{"seq":2,"kind":"snapshot_end"}"#), Instant::now()),
        Err(FeedViolation::UnopenedSnapshot)
    );
}

#[test]
fn feed_age_counts_from_the_last_event_of_any_kind() {
    let table = enabled_table();
    let applied = Instant::now();
    table
        .apply(&event(r#"{"seq":5,"kind":"heartbeat"}"#), applied)
        .expect("applies");

    assert_eq!(
        table.feed_age(applied + Duration::from_secs(7)),
        Duration::from_secs(7)
    );
    assert!(table.has_received());
}
