use std::{collections::BTreeMap, time::Duration};

use super::Tree;
use soma::OciDigest;

use crate::{
    RootfsLimits,
    normalize::entry::{Metadata, PlannedNode},
    normalize::layer::LayerPlan,
};

#[test]
fn ten_thousand_unrelated_entries_avoid_quadratic_full_tree_scans() {
    let mut additions = BTreeMap::new();
    for index in 0..10_000 {
        additions.insert(
            format!("entry-{index:05}").into_bytes(),
            PlannedNode::Directory(Metadata::implicit_directory()),
        );
    }
    let limits = RootfsLimits {
        max_entries: 10_001,
        ..RootfsLimits::default()
    };
    let mut tree = Tree::new(limits).unwrap();
    let started = std::time::Instant::now();
    tree.apply(crate::normalize::layer::LayerPlan {
        additions,
        whiteouts: Vec::new(),
    })
    .unwrap();

    assert_eq!(tree.entries.len(), 10_001);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn long_reverse_hardlink_chain_resolves_in_bounded_time() {
    const LINK_COUNT: usize = 20_000;
    let mut additions = BTreeMap::new();
    additions.insert(
        b"base".to_vec(),
        PlannedNode::Regular {
            metadata: Metadata::implicit_directory(),
            digest: OciDigest::parse(format!("sha256:{}", "11".repeat(32))).unwrap(),
            size: 1,
        },
    );
    for index in 0..LINK_COUNT {
        let target = if index + 1 == LINK_COUNT {
            b"base".to_vec()
        } else {
            format!("link-{:05}", index + 1).into_bytes()
        };
        additions.insert(
            format!("link-{index:05}").into_bytes(),
            PlannedNode::Hardlink { target },
        );
    }
    let limits = RootfsLimits {
        max_entries: u32::try_from(LINK_COUNT + 2).unwrap(),
        ..RootfsLimits::default()
    };
    let mut tree = Tree::new(limits).unwrap();
    let started = std::time::Instant::now();
    tree.apply(LayerPlan {
        additions,
        whiteouts: Vec::new(),
    })
    .unwrap();

    assert_eq!(tree.entries.len(), LINK_COUNT + 2);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn hardlink_target_below_another_pending_hardlink_is_rejected() {
    let mut tree = Tree::new(RootfsLimits::default()).unwrap();
    let mut lower = BTreeMap::new();
    lower.insert(b"old".to_vec(), regular_node());
    lower.insert(b"tree/target".to_vec(), regular_node());
    tree.apply(LayerPlan {
        additions: lower,
        whiteouts: Vec::new(),
    })
    .unwrap();
    let mut additions = BTreeMap::new();
    additions.insert(
        b"tree".to_vec(),
        PlannedNode::Hardlink {
            target: b"old".to_vec(),
        },
    );
    additions.insert(
        b"alias".to_vec(),
        PlannedNode::Hardlink {
            target: b"tree/target".to_vec(),
        },
    );

    assert!(
        tree.apply(LayerPlan {
            additions,
            whiteouts: Vec::new(),
        })
        .is_err()
    );
}

fn regular_node() -> PlannedNode {
    PlannedNode::Regular {
        metadata: Metadata::implicit_directory(),
        digest: OciDigest::parse(format!("sha256:{}", "22".repeat(32))).unwrap(),
        size: 1,
    }
}
