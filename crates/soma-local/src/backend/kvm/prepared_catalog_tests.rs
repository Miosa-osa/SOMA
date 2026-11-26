//! What the prepared catalog lists: every entry a lookup would consider, each with the lookup's
//! own answer, so an entry that would not launch is named rather than missing.

use std::path::{Path, PathBuf};

use super::{CANDIDATE, GENERATION_ID, INCOMING, REFERENCE, STORE_DIRECTORY, catalog_in};

fn scratch(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "soma-catalog-{label}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&root).ok();
    std::fs::create_dir_all(&root).expect("create the test root");
    root
}

/// An entry with bytes that are present but not a real Candidate.
fn entry(root: &Path, name: &str, reference: &str, certified: bool) {
    let entry = root.join(name);
    std::fs::create_dir_all(entry.join(STORE_DIRECTORY)).expect("create the entry");
    std::fs::write(entry.join(REFERENCE), reference).expect("write the reference");
    std::fs::write(entry.join(CANDIDATE), b"not a candidate").expect("write bytes");
    if certified {
        let id = format!("sha256:{}", "ab".repeat(32));
        std::fs::write(entry.join(GENERATION_ID), id).expect("write the id");
    }
}

#[test]
fn every_entry_is_listed_with_the_answer_a_launch_would_get() {
    let root = scratch("answers");
    entry(&root, "ref-b", "node:22", false);
    entry(&root, "ref-a", "miosa-computer:small-4", true);

    let listed = catalog_in(&root).expect("the root is readable");
    let answers: Vec<_> = listed
        .iter()
        .map(|summary| (summary.reference.as_str(), summary.refusal))
        .collect();
    assert_eq!(
        answers,
        [
            ("miosa-computer:small-4", Some("damaged")),
            ("node:22", Some("uncertified")),
        ]
    );
    assert!(listed.iter().all(|summary| summary.ceiling_mib.is_none()));
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn staging_and_entries_without_a_reference_are_not_listed() {
    let root = scratch("staging");
    entry(&root.join(INCOMING), "ref-x", "half:1", true);
    std::fs::create_dir_all(root.join("ref-empty")).expect("an entry with no reference");
    assert_eq!(catalog_in(&root).expect("the root is readable"), []);
    std::fs::remove_dir_all(root).ok();
}

#[test]
fn an_unreadable_root_is_an_error_not_an_empty_list() {
    let root = scratch("missing").join("absent");
    assert_eq!(
        catalog_in(&root),
        Err(crate::CatalogError::StoreUnavailable)
    );
}
