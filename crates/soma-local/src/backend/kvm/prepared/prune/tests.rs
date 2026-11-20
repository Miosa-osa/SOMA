//! A prune against real directories and a cache holding real open files.
//!
//! The cache holds a stand-in value that owns one open descriptor into its entry, so a test can see
//! exactly what an applied prune releases and what a dry run leaves alone.

mod fixture;

use std::{fs::File, sync::Weak};

use soma::InstanceId;

use super::survey::HOLDING;
use crate::{InstanceHolds, PruneReason};
use fixture::{Host, generation, nobody, tree, verdict, write_entry};

#[test]
fn a_pinned_entry_is_never_removed() {
    let host = Host::new();
    let pinned = host.certify("soma-default:G10", 1, 40);
    host.certify("soma-default:G10-b2", 2, 30);
    host.certify("soma-default:G10-sr1", 3, 20);
    host.certify("soma-default:G10-stepa", 4, 10);

    let plan = host.run(&["soma-default:G10"], true, &mut nobody());

    assert!(plan.applied);
    assert_eq!(verdict(&plan, "ref-01"), ("keep", PruneReason::Pinned));
    assert!(
        pinned.join("generation.id").is_file(),
        "the pinned entry is whole"
    );
    assert_eq!(
        verdict(&plan, "ref-04"),
        ("keep", PruneReason::NewerThanPinned)
    );
    for removed in ["ref-02", "ref-03"] {
        assert_eq!(verdict(&plan, removed), ("remove", PruneReason::Superseded));
        assert!(!host.root().join(removed).exists());
    }
}

#[test]
fn an_entry_with_instances_is_refused() {
    let host = Host::new();
    let held = host.certify("soma-default:G20", 1, 30);
    host.certify("soma-default:G20-b2", 2, 20);
    host.certify("soma-default:G20-sr1", 3, 10);
    let instance = InstanceId::new("a".repeat(32)).expect("instance");
    let holds = InstanceHolds {
        held: vec![(instance.clone(), generation(1))],
        unattributed: Vec::new(),
    };

    let plan = host.run(&["node:22"], true, &mut || Ok(holds.clone()));

    assert_eq!(
        verdict(&plan, "ref-01"),
        ("refused", PruneReason::HasInstances)
    );
    assert_eq!(plan.refused[0].detail, [instance.as_str()]);
    assert!(
        held.join("generation.id").is_file(),
        "the held entry is whole"
    );
    assert_eq!(
        verdict(&plan, "ref-02"),
        ("remove", PruneReason::Superseded)
    );
    assert_eq!(plan.unmatched_keep, ["node:22"]);
}

#[test]
fn the_dry_run_changes_nothing_on_disk_or_in_the_cache() {
    let host = Host::new();
    let references = [
        "soma-default:G40",
        "soma-default:G40-b2",
        "soma-default:G40-sr1",
    ];
    let handles: Vec<Weak<File>> = (1_u8..)
        .zip(references)
        .map(|(fill, reference)| {
            let age = 40 - 10 * u64::from(fill);
            host.cache(reference, &host.certify(reference, fill, age))
        })
        .collect();
    let before = tree(&host.root());

    let plan = host.run(&[], false, &mut nobody());

    assert!(!plan.applied);
    assert_eq!(
        plan.remove.len(),
        2,
        "the plan names what an applied run removes"
    );
    assert_eq!(tree(&host.root()), before, "nothing on disk moved");
    assert!(!host.root().join(HOLDING).exists());
    for (reference, handle) in references.into_iter().zip(&handles) {
        assert!(
            host.cache.get(&host.key(reference)).is_some(),
            "still cached"
        );
        assert_eq!(
            handle.strong_count(),
            1,
            "the cache alone still holds it open"
        );
    }
}

#[test]
fn an_applied_prune_releases_the_cached_handles_then_removes_the_entry() {
    let host = Host::new();
    let old = host.certify("node:22", 1, 20);
    host.certify("node:22-c35b568", 2, 10);
    let handle = host.cache("node:22", &old);

    let plan = host.run(&["soma-browser-default:1"], true, &mut nobody());

    assert_eq!(
        verdict(&plan, "ref-01"),
        ("remove", PruneReason::Superseded)
    );
    assert!(
        handle.upgrade().is_none(),
        "the cached descriptor is closed"
    );
    assert!(host.cache.get(&host.key("node:22")).is_none());
    assert!(!old.exists());
    let holding = std::fs::read_dir(host.root().join(HOLDING)).expect("holding directory");
    assert_eq!(holding.count(), 0, "nothing waits for deletion");
}

#[test]
fn an_entry_held_outside_the_cache_is_refused_in_both_runs() {
    let host = Host::new();
    let old = host.certify("node:22", 1, 20);
    host.certify("node:22-c35b568", 2, 10);
    host.cache("node:22", &old);
    let launch = host
        .cache
        .get(&host.key("node:22"))
        .expect("a launch's clone");

    for apply in [false, true] {
        let plan = host.run(&["soma-browser-default:1"], apply, &mut nobody());
        assert_eq!(verdict(&plan, "ref-01"), ("refused", PruneReason::InUse));
    }
    assert!(old.join("generation.id").is_file());
    assert!(host.cache.get(&host.key("node:22")).is_some());
    drop(launch);
}

#[test]
fn a_launch_recorded_during_the_apply_puts_its_entry_back_whole() {
    let host = Host::new();
    let old = host.certify("node:22", 1, 20);
    host.certify("node:22-c35b568", 2, 10);
    let handle = host.cache("node:22", &old);
    let instance = InstanceId::new("b".repeat(32)).expect("instance");
    let mut looks = 0;
    let mut instances = || {
        looks += 1;
        // The first look finds nothing; by the second a launch has recorded itself.
        Ok(InstanceHolds {
            held: if looks == 1 {
                Vec::new()
            } else {
                vec![(instance.clone(), generation(1))]
            },
            unattributed: Vec::new(),
        })
    };

    let plan = host.run(&["soma-browser-default:1"], true, &mut instances);

    assert_eq!(
        verdict(&plan, "ref-01"),
        ("refused", PruneReason::HasInstances)
    );
    assert!(
        old.join("store/root.erofs").is_file(),
        "back at its own path"
    );
    assert!(handle.upgrade().is_some(), "and cached again, still open");
    assert!(host.cache.get(&host.key("node:22")).is_some());
}

#[test]
fn an_interrupted_removal_is_finished_by_the_next_applied_prune() {
    let host = Host::new();
    let leftover = host.root().join(HOLDING).join("ref-09.1700000000");
    write_entry(&leftover, "soma-default:G10-b2", 9, 30);
    let name = format!("{HOLDING}/ref-09.1700000000");

    let plan = host.run(&[], false, &mut nobody());
    assert_eq!(verdict(&plan, &name), ("remove", PruneReason::Interrupted));
    assert!(leftover.exists(), "a dry run deletes nothing");

    let plan = host.run(&["node:22"], true, &mut nobody());
    assert_eq!(verdict(&plan, &name), ("remove", PruneReason::Interrupted));
    assert!(!leftover.exists());
}

#[test]
fn a_named_head_no_instance_owns_refuses_every_removal() {
    let host = Host::new();
    let old = host.certify("node:22", 1, 20);
    host.certify("node:22-c35b568", 2, 10);
    let shard = host.scratch.path().join("heads/h00");
    std::fs::create_dir_all(&shard).expect("shard");
    std::fs::write(shard.join("c".repeat(32)), b"").expect("leaked head");

    let plan = host.run(&["soma-browser-default:1"], true, &mut nobody());

    assert_eq!(
        verdict(&plan, "ref-01"),
        ("refused", PruneReason::UnattributedHead)
    );
    assert!(old.join("generation.id").is_file());
}
