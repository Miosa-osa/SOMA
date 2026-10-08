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
            r#"{"seq":5,"kind":"tenant_policy","tenant_id":"t","soma":true,"suspended":false,"max_concurrent_share":8,"default_timeout_s":600}"#
        )
        .expect("parses"),
        FeedEvent::TenantPolicy {
            seq: 5,
            tenant_id: "t".to_owned(),
            soma: true,
            suspended: false,
            max_concurrent_share: Some(8),
            default_timeout_s: Some(600),
            max_lifetime_s: None,
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
            r#"{{"seq":3,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":null}}"#
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
fn an_endless_line_or_a_regressed_sequence_ends_the_connection() {
    let table = KeyTable::new();
    let mut reader = LineReader::default();
    assert!(
        reader
            .push(&vec![b'x'; 65 * 1024], &table, Instant::now())
            .is_err()
    );

    let mut reader = LineReader::default();
    reader
        .push(
            b"{\"seq\":5,\"kind\":\"heartbeat\"}\n",
            &table,
            Instant::now(),
        )
        .expect("applies");
    assert!(
        reader
            .push(
                b"{\"seq\":3,\"kind\":\"heartbeat\"}\n",
                &table,
                Instant::now()
            )
            .is_err()
    );
}

/// The live finding of 10-03: one `rate_per_s: 10.0` made the whole feed unusable.
#[test]
fn a_float_rate_is_accepted_and_a_malformed_event_is_skipped_not_fatal() {
    let table = KeyTable::new();
    let stream = [
        r#"{"seq":1,"kind":"snapshot_begin"}"#.to_owned(),
        format!(
            r#"{{"seq":2,"kind":"key_upsert","key_hash":"{}","key_id":"k-1","tenant_id":"{TENANT}","user_id":null,"projects":"*","rate_per_s":10.0}}"#,
            hash_of(TOKEN)
        ),
        "{oops}".to_owned(),
        r#"{"seq":3,"kind":"key_upsert","key_hash":"ab"}"#.to_owned(),
        r#"{"seq":4,"kind":"key_upsert","key_hash":"NOT-HEX","key_id":"k-2","tenant_id":"t","user_id":null,"projects":"*","rate_per_s":null}"#.to_owned(),
        format!(
            r#"{{"seq":5,"kind":"tenant_policy","tenant_id":"{TENANT}","soma":true,"suspended":false,"max_concurrent_share":null,"default_timeout_s":300}}"#
        ),
        r#"{"seq":6,"kind":"snapshot_end"}"#.to_owned(),
    ]
    .join("\n")
        + "\n";
    let mut reader = LineReader::default();
    let applied = reader
        .push(stream.as_bytes(), &table, Instant::now())
        .expect("a malformed event does not end the feed");

    assert_eq!(applied, 4, "begin, upsert, policy, end");
    assert_eq!(
        table.last_seq(),
        6,
        "skipped lines still count for the sequence"
    );
    let principal = table
        .admit(Some(&format!("Bearer {TOKEN}")))
        .expect("the key from the float-rate event is served");
    assert_eq!(principal.key.rate_per_second, Some(10));
}

#[test]
fn fractional_and_tiny_rates_round_up_to_at_least_one() {
    for (rate, expected) in [("2.2", 3), ("0.1", 1), ("0", 1), ("-4", 1), ("7", 7)] {
        let event = FeedEvent::parse(&format!(
            r#"{{"seq":1,"kind":"key_upsert","key_hash":"ab","key_id":"k","tenant_id":"t","user_id":null,"projects":"*","rate_per_s":{rate}}}"#
        ))
        .expect("parses");
        let FeedEvent::KeyUpsert { rate_per_s, .. } = event else {
            panic!("a key_upsert");
        };
        assert_eq!(rate_per_s, Some(expected), "rate {rate}");
    }
}

/// A revoke that cannot be applied fails closed: the stream ends, the next connection asks for
/// a full snapshot (`after=0`), and creates are refused until that snapshot has ended.
#[test]
fn an_unappliable_revoke_forces_a_full_resync() {
    for bad in [
        r#"{"seq":5,"kind":"key_revoke"}"#,
        r#"{"seq":5,"kind":"key_revoke","key_hash":"NOT-HEX"}"#,
    ] {
        let table = crate::runner::keys::tests::enabled_table();
        let mut reader = LineReader::default();
        let result = reader.push(format!("{bad}\n").as_bytes(), &table, Instant::now());

        assert!(result.is_err(), "{bad} ends the stream");
        assert!(table.resync_required(), "{bad} refuses creates");
        assert_eq!(table.last_seq(), 0, "{bad} reconnects with after=0");
        assert!(
            table.admit(Some(&format!("Bearer {TOKEN}"))).is_ok(),
            "existing access keeps working until the snapshot replaces it"
        );

        table.begin_connection();
        let mut reader = LineReader::default();
        reader
            .push(
                b"{\"seq\":40,\"kind\":\"snapshot_begin\"}\n",
                &table,
                Instant::now(),
            )
            .expect("applies");
        assert!(table.resync_required(), "still refused mid-snapshot");
        reader
            .push(
                b"{\"seq\":41,\"kind\":\"snapshot_end\"}\n",
                &table,
                Instant::now(),
            )
            .expect("applies");
        assert!(!table.resync_required(), "the snapshot ended the resync");
        assert!(
            table.admit(Some(&format!("Bearer {TOKEN}"))).is_err(),
            "the key the snapshot no longer lists is gone"
        );
    }
}
