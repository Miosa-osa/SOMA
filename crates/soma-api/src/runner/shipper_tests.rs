use super::{Resume, Source, next_batch, resume};
use crate::runner::{
    journal::{
        Journal,
        tests::{entry, scratch, wait_for},
    },
    journal_reader::{AckFile, ShippedLine, epoch_files},
};

fn batch(offsets: std::ops::RangeInclusive<u64>) -> Vec<ShippedLine> {
    offsets
        .map(|offset| ShippedLine {
            offset,
            position: offset * 100,
            bytes: Vec::new(),
        })
        .collect()
}

#[test]
fn a_whole_acknowledgement_continues() {
    assert_eq!(resume(&batch(5..=9), 9), Resume::Continue);
}

#[test]
fn a_partial_acknowledgement_resends_from_the_next_offset() {
    assert_eq!(resume(&batch(5..=9), 6), Resume::SeekTo(700));
    assert_eq!(resume(&batch(5..=9), 4), Resume::SeekTo(500));
}

#[test]
fn an_acknowledgement_outside_the_batch_rescans() {
    assert_eq!(resume(&batch(5..=9), 2), Resume::Rescan);
    assert_eq!(resume(&batch(5..=9), 12), Resume::Rescan);
    assert_eq!(resume(&[], 3), Resume::Rescan);
}

#[test]
fn a_delivered_earlier_epoch_is_deleted_and_the_next_one_ships() {
    let directory = scratch("epochs");
    let old_epoch = {
        let journal = Journal::open(&directory, "r").expect("opens");
        journal.record(entry(201));
        journal.record(entry(200));
        wait_for(&journal, 2);
        AckFile::beside(journal.path()).store(2).expect("acked");
        journal.epoch().to_owned()
    };
    let journal = Journal::open(&directory, "r").expect("reopens");
    journal.record(entry(200));
    wait_for(&journal, 1);
    let source = Source {
        directory: directory.clone(),
        runner: "r".to_owned(),
        current_epoch: journal.epoch().to_owned(),
        written: journal.written_offset(),
        batch_lines: 10,
    };

    let (target, batch) = next_batch(None, &source).expect("next batch");

    assert_eq!(target.expect("current epoch").epoch, journal.epoch());
    assert_eq!(
        batch.iter().map(|line| line.offset).collect::<Vec<_>>(),
        vec![1]
    );
    let left: Vec<String> = epoch_files(&directory, "r")
        .expect("listed")
        .into_iter()
        .map(|(epoch, _)| epoch)
        .collect();
    assert_eq!(left, vec![journal.epoch().to_owned()]);
    assert!(!left.contains(&old_epoch));
    assert!(!directory.join(format!("r.{old_epoch}.acked")).exists());
}

#[test]
fn an_earlier_epoch_with_unacknowledged_lines_ships_first() {
    let directory = scratch("epochs-first");
    let old_epoch = {
        let journal = Journal::open(&directory, "r").expect("opens");
        journal.record(entry(201));
        journal.record(entry(200));
        wait_for(&journal, 2);
        AckFile::beside(journal.path()).store(1).expect("acked");
        journal.epoch().to_owned()
    };
    let journal = Journal::open(&directory, "r").expect("reopens");
    let source = Source {
        directory,
        runner: "r".to_owned(),
        current_epoch: journal.epoch().to_owned(),
        written: 0,
        batch_lines: 10,
    };

    let (target, batch) = next_batch(None, &source).expect("next batch");

    assert_eq!(target.expect("old epoch").epoch, old_epoch);
    assert_eq!(
        batch.iter().map(|line| line.offset).collect::<Vec<_>>(),
        vec![2]
    );
}
