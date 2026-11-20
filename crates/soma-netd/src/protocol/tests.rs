use super::*;
use crate::{EgressClass, ProfileDigest};
use soma_guest::ActivationChallenge;

fn receipt() -> ActivationReceipt {
    let mut bytes = [7_u8; ActivationReceipt::LEN];
    bytes[0] = 1;
    ActivationReceipt::from_bytes(&bytes).expect("receipt")
}

#[test]
fn an_activate_frame_without_a_receipt_is_rejected() {
    let bundle = BundleId::new([3; 16]).expect("id");
    let generation = CleanupGeneration::new(2).expect("g");
    let mut legacy = Vec::with_capacity(21);
    legacy.push(2);
    legacy.extend_from_slice(bundle.as_bytes());
    legacy.extend_from_slice(&generation.get().to_be_bytes());

    assert_eq!(legacy.len(), 21);
    assert_eq!(Request::decode(&legacy), Err(Error::Protocol("request")));

    let mut zero_receipt = legacy.clone();
    zero_receipt.extend_from_slice(&[0; ActivationReceipt::LEN]);
    assert_eq!(
        Request::decode(&zero_receipt),
        Err(Error::Protocol("receipt"))
    );
    const { assert!(21 + ActivationReceipt::LEN <= MAX_FRAME) };
}

#[test]
#[allow(clippy::too_many_lines)]
fn requests_and_replies_round_trip_and_reject_hostile_frames() {
    let intent = NetworkIntent::new(
        EgressClass::Denied,
        Vec::new(),
        Vec::new(),
        ProfileDigest([1; 32]),
    )
    .expect("intent");
    let bundle = BundleId::new([3; 16]).expect("id");
    let generation = CleanupGeneration::new(2).expect("g");
    let requests = [
        Request::Claim {
            instance: InstanceId::new([1; 16]).expect("id"),
            operation: OperationId::new([2; 16]).expect("id"),
            vsock_cid: 7,
            intent,
        },
        Request::Activate {
            bundle,
            generation,
            receipt: receipt(),
        },
        Request::Release { bundle, generation },
        Request::Reconcile,
        Request::Connect {
            instance: InstanceId::new([1; 16]).expect("id"),
            bundle,
            generation,
            port: 9223,
        },
        Request::SetEgress {
            instance: InstanceId::new([1; 16]).expect("id"),
            bundle,
            generation,
            egress: EgressClass::PublicInternet,
            dns: true,
        },
        Request::SetEgress {
            instance: InstanceId::new([1; 16]).expect("id"),
            bundle,
            generation,
            egress: EgressClass::Denied,
            dns: false,
        },
    ];
    for request in requests {
        let encoded = request.encode();
        assert!(encoded.len() <= MAX_FRAME);
        assert_eq!(Request::decode(&encoded).expect("decodes"), request);
        let mut extended = encoded.clone();
        extended.push(0);
        assert!(Request::decode(&extended).is_err());
    }
    let replies = [
        Reply::Claimed {
            bundle,
            generation,
            launch: [9; 35],
            activation: ActivationChallenge::from_bytes([4; 32]).expect("challenge"),
        },
        Reply::Activated,
        Reply::Connected { bundle, generation },
        Reply::EgressSet {
            bundle,
            generation,
            open: true,
            dns: true,
            changed: false,
        },
        Reply::Released { complete: true },
        Reply::Reconciled {
            consistent: 1,
            drifted: 2,
            orphaned: 3,
            unowned: 4,
        },
        Reply::Failed(error_code(&Error::PoolExhausted)),
    ];
    for reply in replies {
        assert_eq!(Reply::decode(&reply.encode()).expect("decodes"), reply);
    }
    assert_eq!(Request::decode(&[]), Err(Error::Protocol("frame length")));
    // A runtime change can name only Denied or PublicInternet, and a flag is 0 or 1.
    let set = Request::SetEgress {
        instance: InstanceId::new([1; 16]).expect("id"),
        bundle,
        generation,
        egress: EgressClass::Denied,
        dns: false,
    }
    .encode();
    let mut unrestricted = set.clone();
    unrestricted[37] = EgressClass::Unrestricted.code();
    assert_eq!(
        Request::decode(&unrestricted),
        Err(Error::Protocol("egress"))
    );
    let mut unknown = set.clone();
    unknown[37] = 0;
    assert_eq!(Request::decode(&unknown), Err(Error::Protocol("egress")));
    let mut bad_flag = set;
    bad_flag[38] = 2;
    assert_eq!(Request::decode(&bad_flag), Err(Error::Protocol("dns")));
    assert_eq!(Request::decode(&[9]), Err(Error::Protocol("request")));
    assert_eq!(Reply::decode(&[3, 2]), Err(Error::Protocol("reply")));
    assert_eq!(
        Request::decode(&[1; 38]).expect_err("bad intent"),
        Error::Protocol("short")
    );
}
