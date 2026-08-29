use std::{
    sync::{Arc, Barrier},
    thread,
};

use super::*;

fn slot() -> Arc<Slot> {
    Slot::new(
        WorkerId::new([7; 16]).expect("id"),
        PoolKeyDigest::from_bytes([1; 32]),
    )
}

#[test]
fn table_admits_exactly_the_specified_transitions() {
    let legal = [
        (Phase::Constructing, Phase::Sterile),
        (Phase::Constructing, Phase::Dead),
        (Phase::Sterile, Phase::Claiming),
        (Phase::Sterile, Phase::Destroying),
        (Phase::Claiming, Phase::Assigned),
        (Phase::Claiming, Phase::Destroying),
        (Phase::Assigned, Phase::Running),
        (Phase::Assigned, Phase::Destroying),
        (Phase::Running, Phase::Destroying),
        (Phase::Destroying, Phase::Dead),
    ];
    for from in Phase::ALL {
        for to in Phase::ALL {
            assert_eq!(
                from.may_transition_to(to),
                legal.contains(&(from, to)),
                "{from:?} -> {to:?}"
            );
        }
        assert_eq!(Phase::from_code(from.code()), Some(from));
    }
    assert!(!Phase::Dead.may_transition_to(Phase::Sterile));
    assert!(!Phase::Assigned.may_transition_to(Phase::Sterile));
    assert!(Phase::Destroying.is_terminal() && Phase::Dead.is_terminal());
    assert!(Phase::Destroying.is_nonterminal() && !Phase::Dead.is_nonterminal());
}

#[test]
fn typestate_handles_walk_the_full_lifecycle_and_bump_only_on_claim() {
    let slot = slot();
    let constructing = Worker::<Constructing>::open(Arc::clone(&slot));
    assert_eq!(constructing.phase(), Phase::Constructing);
    let sterile = constructing.sterilize().expect("sterile");
    assert_eq!(slot.observe().phase, Phase::Sterile);
    let claiming = sterile.claim().expect("claim");
    assert_eq!(claiming.generation().get(), 2);
    let assigned = claiming.assign().expect("assign");
    let running = assigned.run().expect("run");
    let destroying = running.destroy().expect("destroy");
    let dead = destroying.finish().expect("dead");
    assert_eq!(dead.generation().get(), 2);
    assert_eq!(
        slot.observe(),
        Packed {
            phase: Phase::Dead,
            generation: LeaseGeneration::new(2).expect("g"),
        }
    );
    assert!(slot.try_claim().is_none());
    assert!(slot.try_evict().is_none());
}

#[test]
fn state_word_rejects_illegal_moves_generation_drift_and_lost_races() {
    let word = StateWord::new(Phase::Sterile, LeaseGeneration::FIRST);
    let sterile = word.load();
    let two = LeaseGeneration::new(2).expect("g");
    let packed = |phase, generation| Packed { phase, generation };
    assert_eq!(
        word.transition(sterile, packed(Phase::Assigned, LeaseGeneration::FIRST)),
        Err(StateRace::Illegal {
            from: Phase::Sterile,
            to: Phase::Assigned,
        })
    );
    assert_eq!(
        word.transition(sterile, packed(Phase::Claiming, LeaseGeneration::FIRST)),
        Err(StateRace::GenerationRule)
    );
    assert_eq!(
        word.transition(sterile, packed(Phase::Destroying, two)),
        Err(StateRace::GenerationRule)
    );
    word.transition(sterile, packed(Phase::Claiming, two))
        .expect("claim");
    assert_eq!(
        word.transition(sterile, packed(Phase::Claiming, two)),
        Err(StateRace::Lost {
            expected: sterile,
            observed: packed(Phase::Claiming, two),
        })
    );
    let dead = StateWord::new(Phase::Dead, two);
    assert_eq!(
        dead.transition(dead.load(), packed(Phase::Sterile, two)),
        Err(StateRace::Illegal {
            from: Phase::Dead,
            to: Phase::Sterile,
        })
    );
}

#[test]
fn exactly_one_claimer_wins_the_slot() {
    for _ in 0..20 {
        let slot = slot();
        Worker::<Constructing>::open(Arc::clone(&slot))
            .sterilize()
            .expect("sterile");
        let barrier = Arc::new(Barrier::new(32));
        let winners: usize = (0..32)
            .map(|_| {
                let slot = Arc::clone(&slot);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    usize::from(slot.try_claim().is_some())
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().expect("thread"))
            .sum();
        assert_eq!(winners, 1);
        assert_eq!(slot.observe().phase, Phase::Claiming);
    }
}
