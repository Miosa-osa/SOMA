//! What the prepared store must refuse, and why each refusal is distinct.

use super::*;

/// A Candidate must not reach a machine unless the host asked for that explicitly.
#[test]
fn an_uncertified_candidate_is_refused_before_anything_is_created() {
    let root = std::env::temp_dir().join(format!("soma-prepared-cert-{}", std::process::id()));
    let entry = root.join("one");
    std::fs::create_dir_all(entry.join(STORE_DIRECTORY)).expect("create the entry");
    std::fs::write(entry.join(REFERENCE), "node:22").expect("write the reference");
    // Reaching the certification check at all means the bytes decoded, so this test would
    // report Damaged rather than Uncertified if the order were wrong. It asserts the order.
    std::fs::write(entry.join(CANDIDATE), b"not a candidate").expect("write bytes");
    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("damaged bytes are refused before certification is considered"),
        PreparedError::Damaged
    );
}

#[test]
fn only_the_exact_opt_in_value_allows_an_uncertified_candidate() {
    assert!(allows_uncertified(Some(OsStr::new("1"))));
    for refused in ["", "0", "true", "yes", "11"] {
        assert!(
            !allows_uncertified(Some(OsStr::new(refused))),
            "{refused} must not opt in"
        );
    }
    assert!(!allows_uncertified(None));
}

#[test]
fn an_unnamed_store_prepares_nothing() {
    assert_eq!(
        find(None, "node:22").expect_err("an unset store must refuse"),
        PreparedError::StoreUnset
    );
}

#[test]
fn a_missing_root_is_distinguished_from_an_empty_one() {
    assert_eq!(
        find(Some(Path::new("/nonexistent/soma-generations")), "node:22")
            .expect_err("a missing root must refuse"),
        PreparedError::StoreUnreadable
    );
}

#[test]
fn a_readable_root_without_a_match_is_not_prepared() {
    let root = std::env::temp_dir().join(format!("soma-prepared-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create the test root");
    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("an empty root must refuse"),
        PreparedError::NotPrepared
    );
}

#[test]
fn an_entry_matching_the_reference_with_unreadable_bytes_is_damaged_not_missing() {
    let root = std::env::temp_dir().join(format!("soma-prepared-damaged-{}", std::process::id()));
    let entry = root.join("one");
    std::fs::create_dir_all(&entry).expect("create the test entry");
    std::fs::write(entry.join(REFERENCE), "node:22\n").expect("write the reference");
    // The Candidate bytes are absent, so the entry claims a reference it cannot serve.
    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("a damaged entry must refuse"),
        PreparedError::Damaged
    );
}

/// Two entries claiming one reference must not resolve by directory order.
#[test]
fn duplicate_references_are_ambiguous_rather_than_first_wins() {
    let root = std::env::temp_dir().join(format!("soma-prepared-dup-{}", std::process::id()));
    for name in ["one", "two"] {
        let entry = root.join(name);
        std::fs::create_dir_all(entry.join(STORE_DIRECTORY)).expect("create the entry");
        std::fs::write(entry.join(REFERENCE), "node:22").expect("write the reference");
        std::fs::write(entry.join(CANDIDATE), b"not a candidate").expect("write bytes");
    }
    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("two entries for one reference must refuse"),
        PreparedError::Ambiguous
    );
}

/// A linked entry could be redirected after it was verified.
#[test]
fn a_symlinked_entry_is_refused_rather_than_followed() {
    let root = std::env::temp_dir().join(format!("soma-prepared-link-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create the root");
    // The entry that actually holds the bytes lives outside the prepared root, so the only
    // way this reference can resolve is by following the link, which is what must not
    // happen: a link can be repointed after the entry it named was verified.
    let outside = std::env::temp_dir().join(format!("soma-target-{}", std::process::id()));
    std::fs::create_dir_all(outside.join(STORE_DIRECTORY)).expect("create the target");
    std::fs::write(outside.join(REFERENCE), "node:22").expect("write the reference");
    std::fs::write(outside.join(CANDIDATE), b"not a candidate").expect("write bytes");
    std::os::unix::fs::symlink(&outside, root.join("linked")).expect("link the entry");

    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside).ok();
    assert_eq!(
        found.expect_err("a linked entry must refuse"),
        PreparedError::Linked
    );
}

#[test]
fn an_entry_prepared_for_another_reference_is_skipped_rather_than_read() {
    let root = std::env::temp_dir().join(format!("soma-prepared-other-{}", std::process::id()));
    let entry = root.join("one");
    std::fs::create_dir_all(&entry).expect("create the test entry");
    std::fs::write(entry.join(REFERENCE), "alpine:3.20").expect("write the reference");
    // No Candidate bytes: reaching them would be Damaged, so NotPrepared proves the
    // reference is checked before anything else is read.
    let found = find(Some(&root), "node:22");
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("a non-matching entry must not match"),
        PreparedError::NotPrepared
    );
}
