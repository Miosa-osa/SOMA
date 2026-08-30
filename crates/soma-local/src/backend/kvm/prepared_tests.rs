//! What the prepared store must refuse, and why each refusal is distinct.
//!
//! Every test passes the certification allowance explicitly rather than reading the environment,
//! so each case exercises the branch its name claims instead of depending on how the process
//! happened to be started.

use super::*;

/// One entry claiming `reference`, with bytes that are present but not a real Candidate.
///
/// The bytes decode to nothing, so any test that reaches the decode reports `Damaged`. A test
/// that expects another outcome is therefore proving the refusal happened before the decode.
fn entry(root: &Path, name: &str, reference: &str) {
    let entry = root.join(name);
    std::fs::create_dir_all(entry.join(STORE_DIRECTORY)).expect("create the entry");
    std::fs::write(entry.join(REFERENCE), reference).expect("write the reference");
    std::fs::write(entry.join(CANDIDATE), b"not a candidate").expect("write bytes");
}

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "soma-prepared-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).expect("create the test root");
    root
}

#[test]
fn an_unnamed_store_prepares_nothing() {
    assert_eq!(
        find(None, "node:22", true).expect_err("an unset store must refuse"),
        PreparedError::StoreUnset
    );
}

#[test]
fn a_missing_root_is_unreadable_rather_than_linked() {
    assert_eq!(
        find(
            Some(Path::new("/nonexistent/soma-generations")),
            "node:22",
            true
        )
        .expect_err("a missing root must refuse"),
        PreparedError::StoreUnreadable
    );
}

/// A linked root is a different operator problem from an absent one.
#[test]
fn a_linked_root_is_refused() {
    let root = scratch("linkedroot");
    let target = scratch("linkedroottarget");
    entry(&target, "one", "node:22");
    let link = root.join("as-root");
    std::os::unix::fs::symlink(&target, &link).expect("link the root");
    let found = find(Some(&link), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&target).ok();
    assert_eq!(
        found.expect_err("a linked root must refuse"),
        PreparedError::Linked
    );
}

#[test]
fn a_readable_root_without_a_match_is_not_prepared() {
    let root = scratch("empty");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("an empty root must refuse"),
        PreparedError::NotPrepared
    );
}

/// The certification refusal must happen before the bytes are read, so it is what a host sees
/// even when the entry it prepared is damaged.
#[test]
fn a_candidate_is_refused_before_it_is_decoded_when_certification_is_not_allowed() {
    let root = scratch("uncert");
    entry(&root, "one", "node:22");
    let found = find(Some(&root), "node:22", false);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("an uncertified Candidate must refuse"),
        PreparedError::Uncertified
    );
}

/// The same entry, with the allowance granted, reaches the decode and reports how it is damaged.
/// Together with the test above this proves the order: certification first, bytes second.
#[test]
fn the_same_entry_reaches_the_decode_once_certification_is_allowed() {
    let root = scratch("allowed");
    entry(&root, "one", "node:22");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("damaged bytes must refuse"),
        PreparedError::Damaged
    );
}

#[test]
fn an_entry_prepared_for_another_reference_is_skipped_rather_than_read() {
    let root = scratch("other");
    entry(&root, "one", "alpine:3.20");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("a non-matching entry must not match"),
        PreparedError::NotPrepared
    );
}

/// Two entries claiming one reference must not resolve by directory order.
#[test]
fn duplicate_references_are_ambiguous_rather_than_first_wins() {
    let root = scratch("dup");
    entry(&root, "one", "node:22");
    entry(&root, "two", "node:22");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("two entries for one reference must refuse"),
        PreparedError::Ambiguous
    );
}

/// Ambiguity is decided before certification, because which bytes were meant is unknown either
/// way and reporting the wrong reason would send an operator to the wrong problem.
#[test]
fn duplicates_are_ambiguous_even_without_the_certification_allowance() {
    let root = scratch("dupuncert");
    entry(&root, "one", "node:22");
    entry(&root, "two", "node:22");
    let found = find(Some(&root), "node:22", false);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("duplicates must refuse"),
        PreparedError::Ambiguous
    );
}

/// A link can be repointed after the entry it named was verified.
#[test]
fn a_symlinked_entry_is_refused_rather_than_followed() {
    let root = scratch("link");
    let outside = scratch("linktarget");
    entry(&outside, "real", "node:22");
    std::os::unix::fs::symlink(outside.join("real"), root.join("linked")).expect("link the entry");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside).ok();
    assert_eq!(
        found.expect_err("a linked entry must refuse"),
        PreparedError::Linked
    );
}

/// An ancestor link redirects everything beneath it, so the final component is not enough.
#[test]
fn a_linked_ancestor_is_refused() {
    let root = scratch("ancestor");
    let outside = scratch("ancestortarget");
    // The real entries live under `outside/real`, and the root reaches them only through a link
    // named `middle`, so every entry found below it is reached through that link.
    std::fs::create_dir_all(outside.join("real")).expect("create the target");
    entry(&outside.join("real"), "one", "node:22");
    std::os::unix::fs::symlink(outside.join("real"), root.join("middle")).expect("link");
    let found = find(Some(&root.join("middle")), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    std::fs::remove_dir_all(&outside).ok();
    assert_eq!(
        found.expect_err("a linked root must refuse"),
        PreparedError::Linked
    );
}

/// An oversized reference file cannot be read, so it must fail the scan closed.
///
/// This previously asserted the opposite, that an unreadable reference simply did not claim
/// anything. That let a damaged entry vanish from ambiguity detection, which is the defect the
/// re-audit named.
#[test]
fn an_oversized_reference_file_fails_the_scan_closed() {
    let root = scratch("huge");
    let one = root.join("one");
    std::fs::create_dir_all(one.join(STORE_DIRECTORY)).expect("create the entry");
    std::fs::write(one.join(REFERENCE), vec![b'a'; 8192]).expect("write a large reference");
    std::fs::write(one.join(CANDIDATE), b"not a candidate").expect("write bytes");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("an oversized reference must fail the scan"),
        PreparedError::Damaged
    );
}

/// An entry whose reference cannot be read must fail the scan, not vanish from it.
///
/// Without this, a damaged second claimant is silently dropped and a reference that is genuinely
/// ambiguous resolves to whichever entry happened to be readable.
#[test]
fn an_undecidable_claimant_fails_the_scan_rather_than_disappearing() {
    let root = scratch("undecidable");
    entry(&root, "good", "node:22");
    // A second entry claiming something unreadable: an oversized reference file.
    let bad = root.join("bad");
    std::fs::create_dir_all(bad.join(STORE_DIRECTORY)).expect("create the entry");
    std::fs::write(bad.join(REFERENCE), vec![b'a'; 8192]).expect("write a large reference");
    let found = find(Some(&root), "node:22", true);
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(
        found.expect_err("an unreadable claimant must fail the scan"),
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

/// A path that cannot be described must count as a link rather than as an ordinary file.
#[test]
fn an_undescribable_path_counts_as_linked() {
    assert!(is_link(Path::new("/nonexistent/soma-prepared-probe")));
}
