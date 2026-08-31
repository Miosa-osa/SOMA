//! The Host Runtime owns every Instance it launches, by identity, until a terminal operation.

mod support;

use soma_hostd::{ClaimError, InstanceError, Launched, MAX_LISTED, Page};
use support::{harness, instance, intent, limits, op};

/// The Instance of one launch, or a panic naming what the Runtime answered instead.
fn live(launched: Launched) -> soma_hostd::InstanceView {
    match launched {
        Launched::Live(view) => view,
        Launched::Replayed { .. } => panic!("the launch delivered a launch page"),
    }
}

#[test]
fn a_launched_instance_is_addressable_by_identity_after_the_launch_call_returns() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let launched = live(harness.runtime.launch(&intent(1)).expect("launch"));
    assert_eq!(launched.instance, instance(1));

    let found = harness
        .runtime
        .get(instance(1))
        .expect("the Instance is owned by the Host, not by the caller");
    assert_eq!(
        found, launched,
        "the lookup names the same worker and lease"
    );
    assert_eq!(
        harness.runtime.get(instance(2)),
        None,
        "an identity the Host never launched is not owned"
    );
}

#[test]
fn a_replayed_launch_returns_the_same_instance_and_never_a_second_one() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let first = live(harness.runtime.launch(&intent(3)).expect("launch"));
    let replayed = live(harness.runtime.launch(&intent(3)).expect("replay"));
    assert_eq!(replayed, first, "the replay received the first Instance");
    assert_eq!(harness.runtime.live(), 1, "no second Instance was created");
    assert_eq!(
        harness
            .pool
            .ledger()
            .claims()
            .expect("claims")
            .iter()
            .filter(|claim| claim.operation == op(3))
            .count(),
        1,
        "the durable record still holds exactly one claim for the operation"
    );
}

#[test]
fn a_changed_intent_under_one_operation_conflicts_without_creating_an_instance() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    live(harness.runtime.launch(&intent(4)).expect("launch"));

    let mut changed = intent(4);
    changed.vsock_cid += 1;
    let Err(InstanceError::Claim(ClaimError::OperationConflict { operation, .. })) =
        harness.runtime.launch(&changed)
    else {
        panic!("a changed intent under the owning operation must conflict");
    };
    assert_eq!(operation, op(4));

    let mut other_instance = intent(4);
    other_instance.instance = instance(40);
    let Err(InstanceError::Claim(ClaimError::OperationConflict { .. })) =
        harness.runtime.launch(&other_instance)
    else {
        panic!("a changed intent under a recorded operation must conflict");
    };
    assert_eq!(harness.runtime.live(), 1, "no second Instance was created");
}

#[test]
fn a_second_operation_may_not_adopt_a_live_instance() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    live(harness.runtime.launch(&intent(5)).expect("launch"));

    let mut second = intent(5);
    second.operation = op(50);
    assert_eq!(
        harness.runtime.launch(&second),
        Err(InstanceError::Occupied {
            instance: instance(5),
            holder: op(5),
            presented: op(50),
        }),
        "one Machine has exactly one owner"
    );
}

#[test]
fn a_listing_enumerates_exactly_the_live_instances_in_bounded_pages() {
    let count = MAX_LISTED + 2;
    let harness = harness(limits(count, count * 2));
    harness.pool.replenish_blocking().expect("replenish");
    for index in 0..count {
        let index = u32::try_from(index).expect("small");
        live(
            harness
                .runtime
                .launch(&intent(100 + index))
                .expect("launch"),
        );
    }

    let first = harness.runtime.list(None);
    assert_eq!(first.instances.len(), MAX_LISTED, "the page is bounded");
    assert!(first.more, "the listing states that more Instances follow");
    let second = harness.runtime.list(first.instances.last().copied());
    assert_eq!(
        second,
        Page {
            instances: (MAX_LISTED..count)
                .map(|index| instance(100 + u32::try_from(index).expect("small")))
                .collect(),
            more: false,
        },
        "the second page holds the remaining Instances and ends the listing"
    );

    let mut listed: Vec<_> = first.instances;
    listed.extend(second.instances);
    let expected: Vec<_> = (0..count)
        .map(|index| instance(100 + u32::try_from(index).expect("small")))
        .collect();
    assert_eq!(
        listed, expected,
        "every live Instance was listed exactly once"
    );
}

#[test]
fn destroying_an_instance_twice_returns_the_same_terminal_receipt() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let launched = live(harness.runtime.launch(&intent(6)).expect("launch"));

    let first = harness.runtime.destroy(instance(6)).expect("destroy");
    assert_eq!(first.worker, launched.worker);
    assert!(first.complete, "teardown and release both completed");
    assert_eq!(
        harness.runtime.get(instance(6)),
        None,
        "the Instance is gone"
    );

    let repeat = harness
        .runtime
        .destroy(instance(6))
        .expect("a repeat is answered from the durable record");
    assert_eq!(repeat, first, "the repeat receives the identical receipt");
    assert_eq!(
        harness.runtime.destroy(instance(60)),
        Err(InstanceError::Unknown(instance(60))),
        "an identity the Host never launched has no terminal evidence to report"
    );
}

#[test]
fn an_instance_whose_worker_is_gone_is_reclaimed_instead_of_being_listed() {
    let harness = harness(limits(2, 6));
    harness.pool.replenish_blocking().expect("replenish");
    let launched = live(harness.runtime.launch(&intent(7)).expect("launch"));

    // The worker is released behind the Runtime's back, which is what a crash or an operator
    // teardown looks like from here: the binding is stale and must not survive it.
    harness.pool.release(launched.worker).expect("release");
    assert_eq!(harness.runtime.get(instance(7)), None);
    assert_eq!(harness.runtime.list(None).instances, Vec::new());
    assert_eq!(harness.runtime.live(), 0);
    let receipt = harness
        .runtime
        .destroy(instance(7))
        .expect("the ledger still proves the terminal disposition");
    assert!(receipt.complete);
}
