use soma_netd::{EgressClass, NetworkIntent, ProfileDigest};

use super::*;
use crate::{InstanceId, LaunchMaterialHandle, LeaseGeneration, MAX_LISTED, OperationId, WorkerId};

#[test]
fn requests_round_trip_and_reject_hostile_frames() {
    let intent = NetworkIntent::new(
        EgressClass::Denied,
        Vec::new(),
        Vec::new(),
        ProfileDigest([1; 32]),
    )
    .expect("intent");
    let worker = WorkerId::new([3; 16]).expect("id");
    let instance = InstanceId::new([2; 16]).expect("id");
    let frame = LaunchFrame {
        operation: OperationId::new([1; 16]).expect("id"),
        instance,
        vsock_cid: 7,
        deadline_nanos: 9,
        launch_material: LaunchMaterialHandle::new([4; 32]).expect("id"),
        intent,
    };
    let requests = [
        Request::Claim(frame.clone()),
        Request::Release { worker },
        Request::Inspect { worker },
        Request::Reconcile,
        Request::Launch(frame),
        Request::Get { instance },
        Request::List { after: None },
        Request::List {
            after: Some(instance),
        },
        Request::Destroy { instance },
    ];
    for request in requests {
        let encoded = request.encode();
        assert!(encoded.len() <= MAX_FRAME);
        assert_eq!(Request::decode(&encoded).expect("decodes"), request);
        let mut extended = encoded.clone();
        extended.push(0);
        assert!(Request::decode(&extended).is_err());
    }
    assert_eq!(Request::decode(&[]), Err(ProtocolError("frame length")));
    assert_eq!(Request::decode(&[9]), Err(ProtocolError("request")));
    assert!(Request::decode(&[2; 17]).is_ok());
    let mut zero_worker = [0; 17];
    zero_worker[0] = 2;
    assert_eq!(Request::decode(&zero_worker), Err(ProtocolError("worker")));
}

#[test]
fn replies_round_trip_and_reject_hostile_frames() {
    let worker = WorkerId::new([3; 16]).expect("id");
    let instance = InstanceId::new([2; 16]).expect("id");
    let generation = LeaseGeneration::new(2).expect("g");
    let replies = [
        Reply::Claimed {
            worker,
            lease_generation: generation,
            launch: [9; 35],
        },
        Reply::Replayed {
            worker,
            lease_generation: generation,
        },
        Reply::Released { complete: true },
        Reply::Inspected {
            phase: 4,
            lease_generation: generation,
        },
        Reply::Reconciled {
            suspects: 1,
            terminated: 2,
            released: 3,
            retained: 4,
        },
        Reply::Launched {
            worker,
            lease_generation: generation,
            launch: [9; 35],
        },
        Reply::Live {
            worker,
            lease_generation: generation,
            phase: 5,
        },
        Reply::Listed {
            instances: vec![instance],
            more: true,
        },
        Reply::Listed {
            instances: Vec::new(),
            more: false,
        },
        Reply::Destroyed {
            worker,
            complete: true,
        },
        Reply::Failed(failure_code(FailureCode::Exhausted)),
    ];
    for reply in replies {
        let encoded = reply.encode();
        assert!(encoded.len() <= MAX_FRAME);
        assert_eq!(Reply::decode(&encoded).expect("decodes"), reply);
    }
    assert_eq!(Reply::decode(&[3, 2]), Err(ProtocolError("reply")));
    let mut zero_generation = Reply::Replayed {
        worker,
        lease_generation: generation,
    }
    .encode();
    zero_generation[17..25].fill(0);
    assert_eq!(
        Reply::decode(&zero_generation),
        Err(ProtocolError("generation"))
    );
    assert_eq!(failure_code(FailureCode::Protocol), 1);
    assert_eq!(failure_code(FailureCode::Invariant), 11);
    assert_eq!(failure_code(FailureCode::Terminated), 13);
}

#[test]
fn a_listing_frame_that_does_not_match_its_own_count_is_refused() {
    let instance = InstanceId::new([7; 16]).expect("id");
    let mut page = Reply::Listed {
        instances: vec![instance],
        more: false,
    }
    .encode();
    page[2] = 2;
    assert_eq!(
        Reply::decode(&page),
        Err(ProtocolError("reply")),
        "a page claiming more identities than it carries is not a shorter page"
    );
    let mut oversized = vec![8, 0, u8::try_from(MAX_LISTED + 1).expect("small")];
    oversized.extend(std::iter::repeat_n(7, (MAX_LISTED + 1) * 16));
    assert_eq!(
        Reply::decode(&oversized),
        Err(ProtocolError("reply")),
        "a page above the bound is refused rather than allocated"
    );
}
