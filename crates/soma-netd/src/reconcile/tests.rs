use super::*;

#[test]
fn only_a_pin_in_our_directory_makes_a_bundle_ours() {
    let report = ReconcileReport {
        unowned_namespaces: vec![
            "0037835f".to_owned(),
            "keep-me".to_owned(),
            "0037835F".to_owned(),
        ],
        unowned_links: vec![
            "sv00a9afa0".to_owned(),
            "sv0037835f".to_owned(),
            "svXYZ".to_owned(),
            "eth0".to_owned(),
        ],
        unowned_tables: vec![
            "somah_0037835f".to_owned(),
            "somap_deadbeef".to_owned(),
            "miosa_fleet_dist".to_owned(),
            "somah_toolongname".to_owned(),
        ],
        ..ReconcileReport::default()
    };
    let (ours, foreign) = split_by_pin(&report);
    assert_eq!(ours.into_iter().collect::<Vec<_>>(), ["0037835f"]);
    assert_eq!(
        foreign.into_iter().collect::<Vec<_>>(),
        ["00a9afa0", "deadbeef"],
        "a link or table with no pin of ours is reported, never reaped"
    );
}

#[test]
fn a_pin_that_cannot_be_inspected_is_left_alone() {
    use std::io::{Error as IoError, ErrorKind};
    let used = BTreeSet::new();
    assert_eq!(
        pin_state(Err(IoError::from(ErrorKind::PermissionDenied)), &used),
        PinState::Unreadable
    );
    assert_eq!(
        pin_state(Err(IoError::from(ErrorKind::Other)), &used),
        PinState::Unreadable
    );
    assert_eq!(
        pin_state(Err(IoError::from(ErrorKind::NotFound)), &used),
        PinState::Gone
    );
    let here = fs::metadata("/proc/self/ns/net").expect("own namespace");
    let mut used = BTreeSet::new();
    used.insert((here.dev(), here.ino()));
    assert_eq!(pin_state(Ok(here), &used), PinState::InUse);
}

#[test]
fn a_proc_that_hides_processes_is_detected() {
    let plain = "26 25 0:5 / /proc rw,nosuid - proc proc rw\n";
    let hidden = "26 25 0:5 / /proc rw,nosuid,hidepid=2,gid=100 - proc proc rw\n";
    let named = "26 25 0:5 / /proc rw,nosuid - proc proc rw,hidepid=invisible\n";
    let off = "26 25 0:5 / /proc rw,nosuid - proc proc rw,hidepid=off\n";
    let other = "30 25 0:40 / /srv rw - xfs /dev/x rw,hidepid=2\n";
    assert!(!proc_hides_processes(plain));
    assert!(proc_hides_processes(hidden));
    assert!(proc_hides_processes(named));
    assert!(!proc_hides_processes(off));
    assert!(!proc_hides_processes(other), "only the /proc mount counts");
}

#[test]
fn counts_partition_dispositions() {
    let bundle = BundleId::new([1; 16]).expect("id");
    let generation = CleanupGeneration::new(1).expect("g");
    let report = ReconcileReport {
        entries: vec![
            (bundle, generation, Disposition::Consistent),
            (bundle, generation, Disposition::Released),
            (bundle, generation, Disposition::Drifted(Drift::TapMissing)),
            (bundle, generation, Disposition::Orphaned),
        ],
        unowned_namespaces: vec!["x".to_owned()],
        unowned_links: Vec::new(),
        unowned_tables: vec!["soma_deadbeef".to_owned()],
    };
    assert_eq!(report.counts(), (2, 1, 1));
    assert_eq!(report.unowned(), 2);
}
