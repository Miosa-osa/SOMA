use super::*;

const CHAIN: [Phase; 9] = [
    Phase::Captured,
    Phase::MaterialAccepted,
    Phase::EntropyRepaired,
    Phase::TransportFresh,
    Phase::IdentityRepaired,
    Phase::NetworkRepaired,
    Phase::Authenticated,
    Phase::Probed,
    Phase::Ready,
];

fn ready() -> Controller<Ready> {
    let (c, ()) = Controller::captured()
        .accept_material(Ok(()))
        .expect("material");
    let (c, ()) = c.repair_entropy(Ok(())).expect("entropy");
    let (c, ()) = c.freshen_transport(Ok(())).expect("transport");
    let (c, ()) = c.repair_identity(Ok(())).expect("identity");
    let (c, ()) = c.repair_network(Ok(())).expect("network");
    let (c, ()) = c.authenticate(Ok(())).expect("authenticate");
    let (c, ()) = c.probe(Ok(())).expect("probe");
    let (c, ()) = c.ready(Ok(())).expect("ready");
    c
}

#[test]
fn the_repair_chain_is_recorded_in_the_exact_order() {
    let ready = ready();

    assert_eq!(ready.ledger().repair_chain(), &CHAIN);
    assert_eq!(ready.ledger().current(), Phase::Ready);
    assert_eq!(ready.ledger().commands(), 0);
    assert_eq!(ready.ledger().fault(), None);
}

#[test]
fn evidence_flows_through_each_transition() {
    let (controller, value) = Controller::captured()
        .accept_material(Ok(41_u8))
        .expect("material");

    assert_eq!(value, 41);
    assert_eq!(controller.ledger().current(), Phase::MaterialAccepted);
}

#[test]
fn ready_and_running_alternate_and_count_commands() {
    let (running, ()) = ready().run(Ok(())).expect("run");
    assert_eq!(running.ledger().current(), Phase::Running);
    let (ready, ()) = running.finish(Ok(())).expect("finish");
    let (running, ()) = ready.run(Ok(())).expect("second run");
    let (ready, ()) = running.finish(Ok(())).expect("second finish");

    assert_eq!(ready.ledger().commands(), 2);
    assert_eq!(ready.ledger().repair_chain(), &CHAIN);
}

#[test]
fn stopping_follows_ready_only() {
    let (stopping, ()) = ready().stop(Ok(())).expect("stop");

    assert_eq!(stopping.ledger().current(), Phase::Stopping);
    assert_eq!(
        stopping.ledger().repair_chain().last(),
        Some(&Phase::Stopping)
    );
}

#[test]
fn a_stage_failure_poisons_with_that_fault() {
    let (c, ()) = Controller::captured()
        .accept_material(Ok(()))
        .expect("material");
    let poisoned = c
        .repair_entropy(Err::<(), _>(Fault::Entropy))
        .expect_err("entropy failure");

    assert_eq!(poisoned.fault(), Fault::Entropy);
    assert_eq!(poisoned.ledger().current(), Phase::Poisoned);
    assert_eq!(
        poisoned.ledger().repair_chain(),
        &[Phase::Captured, Phase::MaterialAccepted]
    );
}

#[test]
fn explicit_poison_is_terminal_from_every_state() {
    let poisoned = ready().poison(Fault::Control);
    assert_eq!(poisoned.fault(), Fault::Control);
    assert_eq!(poisoned.ledger().current(), Phase::Poisoned);

    let poisoned = Controller::captured().poison(Fault::Boot);
    assert_eq!(poisoned.fault(), Fault::Boot);
}

#[test]
fn phase_permits_only_the_fixed_edges() {
    for window in CHAIN.windows(2) {
        assert!(window[0].permits(window[1]));
        assert!(!window[1].permits(window[0]));
    }
    assert!(Phase::Ready.permits(Phase::Running));
    assert!(Phase::Running.permits(Phase::Ready));
    assert!(Phase::Ready.permits(Phase::Stopping));
    assert!(!Phase::Running.permits(Phase::Stopping));
    assert!(!Phase::Captured.permits(Phase::Ready));
    assert!(!Phase::Captured.permits(Phase::Captured));
    assert!(!Phase::Stopping.permits(Phase::Ready));
    for phase in CHAIN {
        assert!(phase.permits(Phase::Poisoned));
    }
    assert!(!Phase::Poisoned.permits(Phase::Poisoned));
    assert!(!Phase::Poisoned.permits(Phase::Captured));
}

#[test]
fn the_runtime_ledger_rejects_a_skipped_phase() {
    let mut ledger = Ledger::new();

    assert_eq!(ledger.advance(Phase::Ready), Err(Fault::Order));
    assert_eq!(ledger.advance(Phase::MaterialAccepted), Ok(()));
    assert_eq!(ledger.advance(Phase::MaterialAccepted), Err(Fault::Order));
    assert_eq!(ledger.current(), Phase::MaterialAccepted);
}

#[test]
fn default_controller_starts_captured() {
    let controller = Controller::<Captured>::default();

    assert_eq!(controller.ledger().current(), Phase::Captured);
    assert_eq!(controller.ledger().repair_chain(), &[Phase::Captured]);
}
