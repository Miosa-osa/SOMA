use std::time::Instant;

use super::{FeedEvent, ProjectScope, Wildcard};
use crate::runner::feed_client::LineReader;
use crate::runner::keys::{
    KeyTable,
    tests::{TENANT, TOKEN, hash_of},
};

#[test]
fn parses_every_contract_event() {
    assert_eq!(
        FeedEvent::parse(r#"{"seq":1,"kind":"snapshot_begin"}"#).expect("parses"),
        FeedEvent::SnapshotBegin { seq: 1 }
    );
    assert_eq!(
        FeedEvent::parse(
            r#"{"seq":2,"kind":"key_upsert","key_hash":"ab","key_id":"k","tenant_id":"t","user_id":null,"projects":"*","rate_per_s":null}"#
        )
        .expect("parses"),
        FeedEvent::KeyUpsert {
            seq: 2,
            key_hash: "ab".to_owned(),
            key_id: "k".to_owned(),
            tenant_id: "t".to_owned(),
            user_id: None,
            projects: ProjectScope::Wildcard(Wildcard::All),
            rate_per_s: None,
        }
    );
    assert_eq!(
        FeedEvent::parse(
            r#"{"seq":3,"kind":"key_upsert","key_hash":"ab","key_id":"k","tenant_id":"t","user_id":"u","projects":["p1","p2"],"rate_per_s":10}"#
        )
        .expect("parses")
        .seq(),
        3
    );
    assert_eq!(
        FeedEvent::parse(r#"{"seq":4,"kind":"key_revoke","key_hash":"ab"}"#).expect("parses"),
        FeedEvent::KeyRevoke {
            seq: 4,
            key_hash: "ab".to_owned()
        }
    );
    assert_eq!(
        FeedEvent::parse(
            r#"{"seq":5,"kind":"tenant_policy","tenant_id":"t","soma":true,"suspended":false,"max_concurrent":8}"#
        )
        .expect("parses"),
        FeedEvent::TenantPolicy {
            seq: 5,
            tenant_id: "t".to_owned(),
            soma: true,
            suspended: false,
            max_concurrent: Some(8),
        }
    );
    assert_eq!(
        FeedEvent::parse(r#"{"seq":6,"kind":"snapshot_end"}"#).expect("parses"),
        FeedEvent::SnapshotEnd { seq: 6 }
    );
    assert_eq!(
        FeedEvent::parse(r#"{"seq":7,"kind":"heartbeat"}"#).expect("parses"),
        FeedEvent::Heartbeat { seq: 7 }
    );
}

#[test]
fn an_unknown_kind_is_kept_for_its_sequence_number() {
    assert_eq!(
        FeedEvent::parse(r#"{"seq":9,"kind":"region_policy","region":"us"}"#).expect("parses"),
        FeedEvent::Unknown { seq: 9 }
    );
}

#[test]
fn a_known_kind_missing_a_field_is_an_error() {
    assert!(FeedEvent::parse(r#"{"seq":2,"kind":"key_revoke"}"#).is_err());
    assert!(FeedEvent::parse(r#"{"seq":2,"kind":"key_upsert","key_hash":"ab"}"#).is_err());
    assert!(FeedEvent::parse(r#"{"kind":"heartbeat"}"#).is_err());
    assert!(FeedEvent::parse(r#"{"seq":2,"kind":"key_upsert","key_hash":"ab","key_id":"k","tenant_id":"t","user_id":null,"projects":"all","rate_per_s":null}"#).is_err());
    assert!(FeedEvent::parse("not json").is_err());
}

#[test]
fn lines_split_across_chunks_are_reassembled() {
    let table = KeyTable::new();
    let stream = format!(
        "{}\n{}\n\n{}\n{}\n",
        r#"{"seq":1,"kind":"snapshot_begin"}"#,
        format_args!(
            r#"{{"seq":2,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":null}}"#,
            hash_of(TOKEN)
        ),
        format_args!(
            r#"{{"seq":3,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent":null}}"#
        ),
        r#"{"seq":4,"kind":"snapshot_end"}"#,
    );
    let mut reader = LineReader::default();
    let mut applied = 0;
    for chunk in stream.as_bytes().chunks(7) {
        applied += reader
            .push(chunk, &table, Instant::now())
            .expect("every line applies");
    }

    assert_eq!(applied, 4);
    assert_eq!(table.last_seq(), 4);
    assert!(table.admit(Some(&format!("Bearer {TOKEN}"))).is_ok());
}

#[test]
fn a_bad_line_or_an_endless_line_ends_the_connection() {
    let table = KeyTable::new();
    let mut reader = LineReader::default();
    assert!(reader.push(b"{oops}\n", &table, Instant::now()).is_err());

    let mut reader = LineReader::default();
    assert!(
        reader
            .push(&vec![b'x'; 65 * 1024], &table, Instant::now())
            .is_err()
    );
}
