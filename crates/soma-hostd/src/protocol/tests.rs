use soma_netd::{EgressClass, ProfileDigest};

use super::*;
use crate::LeaseGeneration;

#[test]
fn requests_and_replies_round_trip_and_reject_hostile_frames() {
    let intent = NetworkIntent::new(
        EgressClass::Denied,
        Vec::new(),
        Vec::new(),
        ProfileDigest([1; 32]),
    )
    .expect("intent");
    let worker = WorkerId::new([3; 16]).expect("id");
    let generation = LeaseGeneration::new(2).expect("g");
    let requests = [
        Request::Claim {
            operation: OperationId::new([1; 16]).expect("id"),
            instance: InstanceId::new([2; 16]).expect("id"),
            vsock_cid: 7,
            deadline_nanos: 9,
            launch_material: LaunchMaterialHandle::new([4; 32]).expect("id"),
            intent,
        },
        Request::Release { worker },
        Request::Inspect { worker },
        Request::Reconcile,
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
        Reply::Failed(failure_code(FailureCode::Exhausted)),
    ];
    for reply in replies {
        assert_eq!(Reply::decode(&reply.encode()).expect("decodes"), reply);
    }
    assert_eq!(Request::decode(&[]), Err(ProtocolError("frame length")));
    assert_eq!(Request::decode(&[9]), Err(ProtocolError("request")));
    assert!(Request::decode(&[2; 17]).is_ok());
    let mut zero_worker = [0; 17];
    zero_worker[0] = 2;
    assert_eq!(Request::decode(&zero_worker), Err(ProtocolError("worker")));
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
}
