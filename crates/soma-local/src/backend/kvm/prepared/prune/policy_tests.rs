//! The keep rule, decided on entries that exist only as facts.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration, SystemTime},
};

use soma::{GenerationId, InstanceId};

use super::{Decision, Holders, Verdict, decide, decide_with, family};
use crate::backend::kvm::prepared::prune::{
    heads::NamedHead,
    survey::{State, Surveyed},
};
use crate::{InstanceHolds, PinSource, PruneReason};

fn generation(fill: u8) -> GenerationId {
    GenerationId::new(format!("sha256:{}", format!("{fill:02x}").repeat(32))).expect("generation")
}

fn certified(reference: &str, fill: u8, age_days: u64) -> Surveyed {
    Surveyed {
        name: format!("ref-{fill:02x}"),
        path: PathBuf::from(format!("/nonexistent/ref-{fill:02x}")),
        reference: Some(reference.to_owned()),
        state: State::Certified {
            id: generation(fill),
            at: SystemTime::UNIX_EPOCH + Duration::from_secs(10_000 * 86_400 - age_days * 86_400),
        },
        bytes: None,
    }
}

fn pinned(references: &[&str]) -> BTreeMap<String, BTreeSet<PinSource>> {
    references
        .iter()
        .map(|reference| ((*reference).to_owned(), BTreeSet::from([PinSource::Keep])))
        .collect()
}

fn nobody() -> Holders {
    Holders::new(&InstanceHolds::default(), Ok(Vec::new()))
}

fn reasons(decisions: &[Decision]) -> Vec<(Verdict, PruneReason)> {
    decisions
        .iter()
        .map(|decision| (decision.verdict, decision.reason))
        .collect()
}

#[test]
fn a_family_is_the_image_and_the_tag_up_to_its_first_dash() {
    assert_eq!(family("soma-default:G10-b2"), "soma-default:G10");
    assert_eq!(family("soma-default:G10-sr1"), "soma-default:G10");
    assert_eq!(family("soma-default:G10"), "soma-default:G10");
    assert_eq!(family("soma-default:G20-stepa"), "soma-default:G20");
    assert_eq!(family("node:22-c35b568"), "node:22");
    assert_eq!(family("soma-browser-default:1"), "soma-browser-default:1");
    assert_eq!(
        family("registry:5000/team/app:3-rc1"),
        "registry:5000/team/app:3"
    );
    // No tag, a registry port that is not a tag, or a digest: its own family.
    assert_eq!(family("registry:5000/team/app"), "registry:5000/team/app");
    assert_eq!(family("node@sha256:abc-def"), "node@sha256:abc-def");
}

#[test]
fn a_pinned_entry_is_kept_however_old_and_the_newest_previous_one_beside_it() {
    let entries = [
        certified("soma-default:G10", 1, 40),
        certified("soma-default:G10-b2", 2, 30),
        certified("soma-default:G10-sr1", 3, 20),
        certified("soma-default:G10-stepa", 4, 10),
    ];
    let decisions = decide(&entries, &pinned(&["soma-default:G10-sr1"]), &nobody());
    assert_eq!(
        reasons(&decisions),
        [
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::PreviousInFamily),
            (Verdict::Keep, PruneReason::Pinned),
            (Verdict::Keep, PruneReason::NewerThanPinned),
        ]
    );
    assert_eq!(decisions[2].detail, ["keep"]);
}

#[test]
fn a_family_with_no_pin_keeps_only_its_newest_entry() {
    let entries = [
        certified("soma-default:G40", 1, 30),
        certified("soma-default:G40-b2", 2, 20),
        certified("soma-default:G40-sr1", 3, 10),
        certified("node:22", 4, 50),
    ];
    let decisions = decide(&entries, &pinned(&[]), &nobody());
    assert_eq!(
        reasons(&decisions),
        [
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::PreviousInFamily),
            (Verdict::Keep, PruneReason::PreviousInFamily),
        ]
    );
}

#[test]
fn entries_the_rule_cannot_classify_are_kept() {
    let mut uncertified = certified("soma-default:G20", 1, 90);
    uncertified.state = State::Uncertified;
    let mut unreadable = certified("soma-default:G20-b2", 2, 80);
    unreadable.state = State::Unreadable("reference: is a link".to_owned());
    let entries = [
        uncertified,
        unreadable,
        certified("soma-default:G20-sr1", 3, 70),
        certified("soma-default:G20-sr1", 4, 60),
        certified("soma-default:G20-x", 5, 10),
    ];
    let decisions = decide(&entries, &pinned(&[]), &nobody());
    assert_eq!(
        reasons(&decisions),
        [
            (Verdict::Keep, PruneReason::Uncertified),
            (Verdict::Keep, PruneReason::Unreadable),
            (Verdict::Keep, PruneReason::Ambiguous),
            (Verdict::Keep, PruneReason::Ambiguous),
            (Verdict::Keep, PruneReason::PreviousInFamily),
        ]
    );
}

#[test]
fn an_entry_an_instance_holds_is_refused_by_name_and_its_head_adds_no_refusal() {
    let entries = [
        certified("soma-default:G10", 1, 30),
        certified("soma-default:G10-b2", 2, 20),
        certified("soma-default:G10-sr1", 3, 10),
        certified("soma-default:G10-z", 4, 5),
    ];
    let live = InstanceId::new("a".repeat(32)).expect("instance");
    let headed = InstanceId::new("b".repeat(32)).expect("instance");
    let holds = InstanceHolds {
        held: vec![(live.clone(), generation(1)), (headed, generation(2))],
        unattributed: Vec::new(),
    };
    let head = NamedHead {
        name: "b".repeat(32),
        path: PathBuf::from("/srv/heads/h03").join("b".repeat(32)),
    };
    let decisions = decide(
        &entries,
        &pinned(&[]),
        &Holders::new(&holds, Ok(vec![head])),
    );
    assert_eq!(decisions[0].reason, PruneReason::HasInstances);
    assert_eq!(decisions[0].detail, [live.as_str()]);
    assert_eq!(decisions[1].reason, PruneReason::HasInstances);
    // The head belongs to a live Instance, so it refuses nothing that Instance does not.
    assert_eq!(decisions[2], Decision::remove(PruneReason::Superseded));
}

#[test]
fn anything_unattributed_refuses_every_removal() {
    let entries = [
        certified("soma-default:G10", 1, 30),
        certified("soma-default:G10-b2", 2, 20),
    ];
    let unknown = InstanceId::new("c".repeat(32)).expect("instance");
    let holds = InstanceHolds {
        held: Vec::new(),
        unattributed: vec![unknown],
    };
    let decisions = decide(
        &entries,
        &pinned(&[]),
        &Holders::new(&holds, Ok(Vec::new())),
    );
    assert_eq!(decisions[0].reason, PruneReason::UnattributedInstance);

    let orphan = NamedHead {
        name: "leaked".to_owned(),
        path: PathBuf::from("/srv/heads/h00/leaked"),
    };
    let holders = Holders::new(&InstanceHolds::default(), Ok(vec![orphan]));
    let decisions = decide(&entries, &pinned(&[]), &holders);
    assert_eq!(decisions[0].reason, PruneReason::UnattributedHead);
    assert_eq!(decisions[0].detail, ["/srv/heads/h00/leaked"]);

    let unreadable = Holders::new(&InstanceHolds::default(), Err("heads: denied".to_owned()));
    let decisions = decide(&entries, &pinned(&[]), &unreadable);
    assert_eq!(decisions[0].reason, PruneReason::UnattributedHead);
}

#[test]
fn the_rollback_target_is_kept_by_default_and_dropped_on_request() {
    let entries = [
        certified("soma-default:G10", 1, 40),
        certified("soma-default:G10-b2", 2, 30),
        certified("soma-default:G10-sr1", 3, 20),
        certified("soma-default:G10-stepa", 4, 10),
    ];
    let pins = pinned(&["soma-default:G10-sr1"]);
    assert_eq!(
        reasons(&decide_with(&entries, &pins, &nobody(), true)),
        [
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::PreviousInFamily),
            (Verdict::Keep, PruneReason::Pinned),
            (Verdict::Keep, PruneReason::NewerThanPinned),
        ]
    );
    // Only the rollback target goes; the entry certified after the pin stays.
    assert_eq!(
        reasons(&decide_with(&entries, &pins, &nobody(), false)),
        [
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::Pinned),
            (Verdict::Keep, PruneReason::NewerThanPinned),
        ]
    );
}

#[test]
fn a_pinned_entry_is_never_dropped_even_without_the_rollback() {
    let entries = [
        certified("soma-default:G10", 1, 40),
        certified("soma-default:G10-b2", 2, 30),
        certified("soma-default:G10-sr1", 3, 20),
    ];
    let pins = pinned(&["soma-default:G10-b2", "soma-default:G10-sr1"]);
    assert_eq!(
        reasons(&decide_with(&entries, &pins, &nobody(), false)),
        [
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::Pinned),
            (Verdict::Keep, PruneReason::Pinned),
        ]
    );
}

#[test]
fn without_the_rollback_a_family_the_keep_list_does_not_name_keeps_its_own() {
    let entries = [
        certified("soma-browser-default:1", 1, 10),
        certified("soma-browser-default:1-old", 2, 20),
        certified("soma-default:G40-b2", 3, 30),
        certified("soma-default:G40-sr1", 4, 20),
    ];
    // Pinned by the host's own prewarm reference, not by the keep list.
    let pins = BTreeMap::from([(
        "soma-browser-default:1".to_owned(),
        BTreeSet::from([PinSource::PrewarmReference]),
    )]);
    assert_eq!(
        reasons(&decide_with(&entries, &pins, &nobody(), false)),
        [
            (Verdict::Keep, PruneReason::Pinned),
            (Verdict::Keep, PruneReason::PreviousInFamily),
            (Verdict::Remove, PruneReason::Superseded),
            (Verdict::Keep, PruneReason::PreviousInFamily),
        ]
    );
}

#[test]
fn an_entry_in_use_is_still_refused_without_the_rollback() {
    let entries = [
        certified("soma-default:G20-b2", 2, 30),
        certified("soma-default:G20-sr1", 3, 20),
    ];
    let holds = InstanceHolds {
        held: vec![(
            InstanceId::new("d".repeat(32)).expect("instance"),
            generation(2),
        )],
        unattributed: Vec::new(),
    };
    let decisions = decide_with(
        &entries,
        &pinned(&["soma-default:G20-sr1"]),
        &Holders::new(&holds, Ok(Vec::new())),
        false,
    );
    assert_eq!(decisions[0].verdict, Verdict::Refuse);
    assert_eq!(decisions[0].reason, PruneReason::HasInstances);
}
