//! Only a repaired authenticated session mints a network-activation capability, and that
//! capability authenticates for exactly one assignment scope.

mod control_support;

use core::convert::Infallible;

use soma_guest::{
    ActivationChallenge, ActivationScope, Error, GuestControl, GuestLaunchMaterial, GuestRequest,
    HostControl, HostLaunchMaterial, LAUNCH_PAGE_SIZE, OperationId, RepairedHostControl,
    TerminalStatus,
};

use control_support::{MemoryIo, deadline, launch_network, pair};

const INSTANCE: [u8; 16] = [0x51; 16];
const OPERATION: [u8; 16] = [0x62; 16];
const GENERATION: u32 = 9;
const INTENT: [u8; 32] = [0x73; 32];

fn repaired() -> RepairedHostControl<MemoryIo> {
    let host = HostLaunchMaterial::generate([0x40; 32], INSTANCE, OPERATION, launch_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    let (host_io, guest_io, _, _) = pair();
    let guest_thread = std::thread::spawn(move || {
        let guest =
            GuestControl::connect(guest, guest_io, deadline()).expect("guest owner connected");
        let (guest, request) = guest.next_request(deadline()).expect("prepare request");
        assert_eq!(
            request,
            GuestRequest::PrepareAndProbe {
                operation: OperationId::new(OPERATION).expect("launch operation"),
            }
        );
        guest
            .repair_complete(deadline())
            .expect("repair complete")
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("probe terminal")
    });
    let repaired = HostControl::connect(host, host_io)
        .expect("host owner connected")
        .prepare_and_probe()
        .expect("authenticated repair and probe");
    drop(guest_thread.join().expect("guest thread"));
    repaired
}

fn scope() -> ActivationScope {
    ActivationScope::new(INSTANCE, OPERATION, GENERATION, INTENT).expect("scope")
}

#[test]
fn a_repaired_session_mints_a_receipt_the_broker_challenge_authenticates() {
    let host = repaired();
    let challenge = ActivationChallenge::generate().expect("broker challenge");

    let receipt = host
        .network_activation(&challenge, GENERATION, INTENT)
        .expect("receipt");

    assert_eq!(challenge.verify(&scope(), &receipt), Ok(()));
    assert_ne!(receipt.transcript(), &[0; 32]);
}

#[test]
fn the_published_transcript_is_the_one_a_repaired_session_binds() {
    let host = repaired();
    let challenge = ActivationChallenge::generate().expect("broker challenge");
    let receipt = host
        .network_activation(&challenge, GENERATION, INTENT)
        .expect("receipt");

    let transcript = host.session_transcript();

    assert_ne!(transcript, [0; 32]);
    assert_eq!(
        &transcript,
        receipt.transcript(),
        "the published transcript is not the one this session authenticated with"
    );
}

#[test]
fn a_receipt_does_not_authenticate_for_another_assignment_or_challenge() {
    let host = repaired();
    let challenge = ActivationChallenge::generate().expect("broker challenge");
    let receipt = host
        .network_activation(&challenge, GENERATION, INTENT)
        .expect("receipt");

    for other in [
        ActivationScope::new([0x99; 16], OPERATION, GENERATION, INTENT).expect("other instance"),
        ActivationScope::new(INSTANCE, [0x98; 16], GENERATION, INTENT).expect("other operation"),
        ActivationScope::new(INSTANCE, OPERATION, GENERATION + 1, INTENT)
            .expect("other generation"),
        ActivationScope::new(INSTANCE, OPERATION, GENERATION, [0x97; 32]).expect("other intent"),
    ] {
        assert_eq!(
            challenge.verify(&other, &receipt),
            Err(Error::ActivationReceiptRejected)
        );
    }

    let other_challenge = ActivationChallenge::generate().expect("other challenge");
    assert_eq!(
        other_challenge.verify(&scope(), &receipt),
        Err(Error::ActivationReceiptRejected)
    );
}

#[test]
fn a_second_session_cannot_reuse_the_first_transcript_binding() {
    let challenge = ActivationChallenge::generate().expect("broker challenge");
    let first = repaired()
        .network_activation(&challenge, GENERATION, INTENT)
        .expect("first receipt");
    let second = repaired()
        .network_activation(&challenge, GENERATION, INTENT)
        .expect("second receipt");

    assert_ne!(first.transcript(), second.transcript());
    assert_ne!(first, second);
    assert_eq!(challenge.verify(&scope(), &first), Ok(()));
    assert_eq!(challenge.verify(&scope(), &second), Ok(()));
}

#[test]
fn a_zero_generation_or_intent_cannot_be_bound() {
    let host = repaired();
    let challenge = ActivationChallenge::generate().expect("broker challenge");

    assert_eq!(
        host.network_activation(&challenge, 0, INTENT),
        Err(Error::InvalidActivationScope)
    );
    assert_eq!(
        host.network_activation(&challenge, GENERATION, [0; 32]),
        Err(Error::InvalidActivationScope)
    );
}
