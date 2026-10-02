use std::{
    io::Write,
    path::PathBuf,
    time::{Duration, Instant},
};

use super::{Entry, EntryKind, Journal};
use crate::runner::journal_reader::{AckFile, Reader};

pub(crate) fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "soma-runner-{name}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

pub(crate) fn entry(status: u16) -> Entry {
    Entry {
        kind: EntryKind::Create,
        tenant_id: "t-1".to_owned(),
        key_id: Some("k-1".to_owned()),
        project_id: None,
        sandbox_id: Some("3f2504e0-4f89-41d3-9a0c-0305e82c3301".to_owned()),
        status,
        ms: 4,
        exit_code: None,
        cpu_ms: None,
        lifetime_ms: None,
    }
}

pub(crate) fn wait_for(journal: &Journal, offset: u64) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while journal.durable_offset() < offset {
        assert!(Instant::now() < deadline, "journal reached {offset}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn lines_carry_contract_fields_in_order_and_increasing_offsets() {
    let directory = scratch("order");
    let journal = Journal::open(&directory, "miosa-host-03").expect("opens");
    journal.record(entry(201));
    journal.record(entry(503));
    wait_for(&journal, 2);

    let text = std::fs::read_to_string(journal.path()).expect("readable");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with(r#"{"runner":"miosa-host-03","offset":1,"ts":""#));
    assert!(lines[0].ends_with(
        r#""kind":"create","tenant_id":"t-1","key_id":"k-1","project_id":null,"sandbox_id":"3f2504e0-4f89-41d3-9a0c-0305e82c3301","status":201,"ms":4,"exit_code":null,"cpu_ms":null,"lifetime_ms":null}"#
    ));
    assert!(lines[1].contains(r#""offset":2,"#));
    let ts: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON");
    assert_eq!(ts["ts"].as_str().expect("ts").len(), 24);
}

#[test]
fn offsets_continue_across_a_restart_and_a_torn_line_is_cut() {
    let directory = scratch("restart");
    {
        let journal = Journal::open(&directory, "r").expect("opens");
        journal.record(entry(201));
        journal.record(entry(200));
        wait_for(&journal, 2);
    }
    let path = directory.join("r.ndjson");
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append")
        .write_all(br#"{"runner":"r","offset":3,"ts":"#)
        .expect("torn write");

    let journal = Journal::open(&directory, "r").expect("reopens");
    assert_eq!(journal.durable_offset(), 2);
    journal.record(entry(200));
    wait_for(&journal, 3);

    let text = std::fs::read_to_string(&path).expect("readable");
    let offsets: Vec<u64> = text
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("every line is whole")["offset"]
                .as_u64()
                .expect("offset")
        })
        .collect();
    assert_eq!(offsets, vec![1, 2, 3]);
}

#[test]
fn the_reader_resumes_after_an_acknowledged_offset_and_stops_at_durable() {
    let directory = scratch("reader");
    let journal = Journal::open(&directory, "r").expect("opens");
    for _ in 0..5 {
        journal.record(entry(200));
    }
    wait_for(&journal, 5);

    let mut reader = Reader::after(journal.path(), 2).expect("reader");
    let batch = reader.batch(10, 4).expect("batch");
    assert_eq!(
        batch.iter().map(|line| line.offset).collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert_eq!(
        reader
            .batch(10, 5)
            .expect("batch")
            .iter()
            .map(|line| line.offset)
            .collect::<Vec<_>>(),
        vec![5]
    );

    reader.seek_to(batch[1].position);
    assert_eq!(reader.batch(1, 5).expect("batch")[0].offset, 4);
}

#[test]
fn the_acknowledgement_survives_a_restart() {
    let directory = scratch("ack");
    let acks = AckFile::beside(&directory.join("r.ndjson"));

    assert_eq!(acks.load().expect("absent is zero"), 0);
    acks.store(41).expect("stores");
    assert_eq!(
        AckFile::beside(&directory.join("r.ndjson"))
            .load()
            .expect("loads"),
        41
    );
}
