use std::path::PathBuf;

use crate::prewarm::{MAX_TARGET_COUNT, MAX_TARGETS, PERSISTED_FILE, PrewarmError, PrewarmTarget};

use super::persist::{load, persist};
use super::resolve::{check_budget, resolve_in};

fn target(reference: &str, count: usize) -> PrewarmTarget {
    PrewarmTarget {
        reference: reference.to_owned(),
        count,
        memory_mib: None,
    }
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "soma-targets-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).expect("create the test root");
    root
}

#[test]
fn a_host_with_no_store_prepares_nothing_and_says_so() {
    assert_eq!(
        resolve_in(None, &[target("node:22", 1)]).err(),
        Some(PrewarmError::Unsupported)
    );
}

#[test]
fn a_reference_with_no_prepared_generation_is_named() {
    let root = scratch("unknown");
    assert_eq!(
        resolve_in(Some(&root), &[target("node:22", 2)]).err(),
        Some(PrewarmError::UnknownReference("node:22".to_owned()))
    );
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn a_malformed_set_is_refused_before_any_reference_is_looked_up() {
    let root = scratch("malformed");
    for (set, reason) in [
        (vec![target("", 1)], "a reference is empty"),
        (vec![target("   ", 1)], "a reference is empty"),
        (vec![target("a", MAX_TARGET_COUNT + 1)], "count is at most"),
        (vec![target("a", 1), target("a", 2)], "appears twice"),
        (
            (0..=MAX_TARGETS)
                .map(|n| target(&format!("r{n}"), 1))
                .collect(),
            "at most",
        ),
    ] {
        match resolve_in(Some(&root), &set) {
            Err(PrewarmError::InvalidTarget(text)) => {
                assert!(text.contains(reason), "{text} should mention {reason}");
            }
            other => panic!("expected an invalid target, got {:?}", other.err()),
        }
    }
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn the_budget_counts_count_times_memory_and_refuses_over_it() {
    assert_eq!(check_budget(&[(10, 4096), (5, 8192)], 100_000), Ok(81_920));
    assert_eq!(check_budget(&[], 0), Ok(0));
    assert_eq!(
        check_budget(&[(10, 4096), (5, 8192)], 81_919),
        Err(PrewarmError::BudgetExceeded {
            budget_mib: 81_919,
            requested_mib: 81_920,
        })
    );
    // A count that would overflow is still a refusal, never a wrap to a small number.
    assert!(check_budget(&[(usize::MAX, u64::MAX)], u64::MAX - 1).is_err());
}

#[test]
fn a_persisted_set_round_trips_and_a_missing_file_is_no_set() {
    let root = scratch("persist");
    assert_eq!(load(&root), Ok(None));
    let set = vec![target("node:22", 20), target("python:3.12", 4)];
    persist(&root, &set).expect("persist");
    assert_eq!(load(&root), Ok(Some(set)));
    // The temporary file does not outlive a successful write.
    assert!(!root.join(format!("{PERSISTED_FILE}.tmp")).exists());
    // A second write replaces the first whole.
    persist(&root, &[]).expect("replace");
    assert_eq!(load(&root), Ok(Some(Vec::new())));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn a_persisted_file_is_private_and_a_damaged_one_is_reported_not_used() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = scratch("damaged");
    persist(&root, &[target("node:22", 1)]).expect("persist");
    let mode = std::fs::metadata(root.join(PERSISTED_FILE))
        .expect("stat")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);

    for bad in [
        &b"not json"[..],
        br#"{"version":2,"targets":[]}"#,
        br#"{"version":1,"targets":[],"extra":1}"#,
        br#"{"version":1,"targets":[{"reference":"a","count":1,"surprise":true}]}"#,
    ] {
        std::fs::write(root.join(PERSISTED_FILE), bad).expect("write");
        assert!(load(&root).is_err(), "{}", String::from_utf8_lossy(bad));
    }
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn a_failed_write_leaves_the_previous_set_in_place() {
    let root = scratch("failed-write");
    persist(&root, &[target("node:22", 3)]).expect("persist");
    // A directory squatting on the temporary name makes the write fail.
    let squat = root.join(format!("{PERSISTED_FILE}.tmp"));
    std::fs::create_dir(&squat).expect("squat");
    // The temporary file is removed first, so this write succeeds; make it fail by pointing
    // at a root that does not exist instead.
    assert!(persist(&root.join("absent"), &[]).is_err());
    assert_eq!(load(&root), Ok(Some(vec![target("node:22", 3)])));
    std::fs::remove_dir_all(root).ok();
}
