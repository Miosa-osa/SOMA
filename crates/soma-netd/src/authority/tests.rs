use super::*;
use crate::{
    BundleId, CleanupGeneration, EgressClass, InstanceId, NetworkIntent, OperationId, ProfileDigest,
};
use soma_guest::ActivationReceipt;

fn requests() -> [Request; 4] {
    let bundle = BundleId::new([3; 16]).expect("bundle");
    let generation = CleanupGeneration::new(2).expect("generation");
    let mut receipt = [7_u8; ActivationReceipt::LEN];
    receipt[0] = 1;
    [
        Request::Claim {
            instance: InstanceId::new([1; 16]).expect("instance"),
            operation: OperationId::new([2; 16]).expect("operation"),
            vsock_cid: 7,
            intent: NetworkIntent::new(
                EgressClass::Denied,
                Vec::new(),
                Vec::new(),
                ProfileDigest([1; 32]),
            )
            .expect("intent"),
        },
        Request::Activate {
            bundle,
            generation,
            receipt: ActivationReceipt::from_bytes(&receipt).expect("receipt"),
        },
        Request::Release { bundle, generation },
        Request::Reconcile,
    ]
}

#[test]
fn every_operation_names_the_capability_it_requires() {
    let [claim, activate, release, reconcile] = requests();

    assert_eq!(Capability::required_for(&claim), Capability::Lifecycle);
    assert_eq!(Capability::required_for(&activate), Capability::Lifecycle);
    assert_eq!(Capability::required_for(&release), Capability::Lifecycle);
    assert_eq!(Capability::required_for(&reconcile), Capability::Reconcile);
}

#[test]
fn an_authority_that_admits_nobody_is_rejected() {
    assert_eq!(
        ControlAuthority::new(0, 0, &[], &[]),
        Err(Error::InvalidProfile("control authority admits nobody"))
    );
}

#[test]
fn each_capability_is_granted_separately_and_unknown_peers_are_refused() {
    let authority = ControlAuthority::new(0, 500, &[1000], &[1001]).expect("authority");
    let host = PeerIdentity::new(1000, 500, 42);
    let operator = PeerIdentity::new(1001, 500, 43);
    let stranger = PeerIdentity::new(1002, 500, 44);

    assert_eq!(authority.owner(), 0);
    assert_eq!(authority.group(), 500);
    assert_eq!(host.uid(), 1000);
    assert_eq!(host.gid(), 500);
    assert_eq!(host.pid(), 42);

    assert!(authority.admits(&host));
    assert!(authority.permits(&host, Capability::Lifecycle));
    assert!(
        !authority.permits(&host, Capability::Reconcile),
        "the lifecycle peer must not reconcile"
    );

    assert!(authority.admits(&operator));
    assert!(authority.permits(&operator, Capability::Reconcile));
    assert!(
        !authority.permits(&operator, Capability::Lifecycle),
        "the operator peer must not claim, activate, or release"
    );

    assert!(!authority.admits(&stranger));
    assert!(!authority.permits(&stranger, Capability::Lifecycle));
    assert!(!authority.permits(&stranger, Capability::Reconcile));
}
