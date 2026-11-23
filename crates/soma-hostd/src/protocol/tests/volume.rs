//! Volume, workspace restore and workspace import frames on the wire.

use super::*;

#[test]
fn a_volume_attach_and_detach_round_trip_and_reject_a_bad_label() {
    let attach = Request::VolumeAttach(AttachFrame {
        operation: OperationId::new([1; 16]).expect("nonzero"),
        instance: InstanceId::new([2; 16]).expect("nonzero"),
        volume: VolumeId::new([3; 16]).expect("nonzero"),
        fingerprint: RequestFingerprint::new([4; 32]).expect("nonzero"),
        size_class_gib: 10,
        generation: GenerationId::new([5; 32]).expect("nonzero"),
        tenant: "tenant-alpha".to_owned(),
    });
    let encoded = attach.encode();
    assert!(encoded.len() <= MAX_FRAME);
    assert_eq!(Request::decode(&encoded).expect("decodes"), attach);

    let detach = Request::VolumeDetach(DetachFrame {
        operation: OperationId::new([1; 16]).expect("nonzero"),
        instance: InstanceId::new([2; 16]).expect("nonzero"),
        volume: VolumeId::new([3; 16]).expect("nonzero"),
        fingerprint: RequestFingerprint::new([4; 32]).expect("nonzero"),
    });
    let encoded = detach.encode();
    assert_eq!(Request::decode(&encoded).expect("decodes"), detach);

    // A label whose padding is not zero is refused rather than read as a longer label.
    let mut hostile = Request::VolumeAttach(AttachFrame {
        operation: OperationId::new([1; 16]).expect("nonzero"),
        instance: InstanceId::new([2; 16]).expect("nonzero"),
        volume: VolumeId::new([3; 16]).expect("nonzero"),
        fingerprint: RequestFingerprint::new([4; 32]).expect("nonzero"),
        size_class_gib: 10,
        generation: GenerationId::new([5; 32]).expect("nonzero"),
        tenant: "ab".to_owned(),
    })
    .encode();
    let last = hostile.len() - 1;
    hostile[last] = 0x7f;
    assert_eq!(
        Request::decode(&hostile),
        Err(ProtocolError("tenant")),
        "a non-zero pad byte is a malformed label"
    );
}

#[test]
fn a_volume_open_round_trips_and_its_path_padding_is_refused() {
    let open = Request::VolumeOpen {
        instance: InstanceId::new([7; 16]).expect("nonzero"),
    };
    let encoded = open.encode();
    assert_eq!(encoded.len(), 17);
    assert_eq!(Request::decode(&encoded).expect("decodes"), open);

    let opened = Reply::VolumeOpened {
        volume: VolumeId::new([3; 16]).expect("nonzero"),
        size_class_gib: 10,
        repair: 1,
        exit_code: 0,
        path: "/srv/soma/volumes/03/volume.img".to_owned(),
    };
    let encoded = opened.encode();
    assert!(encoded.len() <= MAX_FRAME);
    assert_eq!(Reply::decode(&encoded).expect("decodes"), opened);

    // A path whose padding is not zero is refused rather than read as a longer path.
    let mut hostile = Reply::VolumeOpened {
        volume: VolumeId::new([3; 16]).expect("nonzero"),
        size_class_gib: 10,
        repair: 1,
        exit_code: 0,
        path: "ab".to_owned(),
    }
    .encode();
    let last = hostile.len() - 1;
    hostile[last] = 0x7f;
    assert_eq!(
        Reply::decode(&hostile),
        Err(ProtocolError("volume path")),
        "a non-zero pad byte is a malformed path"
    );
}

#[test]
fn workspace_restore_requests_round_trip_with_exact_operation_and_volume() {
    use crate::volume::{CheckpointVolumeRequest, WorkspaceRestoreRequest};

    let instance = InstanceId::new([1; 16]).unwrap();
    let workspace = WorkspaceRestoreRequest {
        checkpoint: CheckpointVolumeRequest {
            source: instance,
            destination: instance,
            checkpoint: OperationId::new([2; 16]).unwrap(),
            generation: GenerationId::new([3; 32]).unwrap(),
            digest: RequestFingerprint::new([4; 32]).unwrap(),
        },
        operation: OperationId::new([5; 16]).unwrap(),
        volume: VolumeId::new([6; 16]).unwrap(),
    };
    for (tag, request) in [
        (19, Request::WorkspaceRestoreStage(workspace)),
        (20, Request::WorkspaceRestorePublish(workspace)),
        (21, Request::WorkspaceRestoreAbort(workspace)),
        (22, Request::WorkspaceRestoreReconcile(workspace)),
    ] {
        let encoded = request.encode();
        assert_eq!(encoded[0], tag);
        assert_eq!(Request::decode(&encoded).unwrap(), request);
        assert!(Request::decode(&encoded[..encoded.len() - 1]).is_err());
        let mut zero_volume = encoded.clone();
        let offset = zero_volume.len() - 16;
        zero_volume[offset..].fill(0);
        assert!(Request::decode(&zero_volume).is_err());
    }
}

#[test]
fn workspace_import_frames_bind_source_and_cold_destination() {
    use crate::{
        AttachRequest, GenerationId, InstanceId, OperationId, RequestFingerprint, VolumeId,
        volume::WorkspaceImportRequest,
    };

    let request = Request::WorkspaceImport(WorkspaceImportRequest {
        source: InstanceId::new([1; 16]).unwrap(),
        checkpoint: OperationId::new([2; 16]).unwrap(),
        digest: RequestFingerprint::new([3; 32]).unwrap(),
        destination: AttachRequest {
            operation: OperationId::new([4; 16]).unwrap(),
            instance: InstanceId::new([5; 16]).unwrap(),
            volume: VolumeId::new([6; 16]).unwrap(),
            fingerprint: RequestFingerprint::new([7; 32]).unwrap(),
            size_class_gib: 10,
            generation: GenerationId::new([8; 32]).unwrap(),
            tenant: "tenant-alpha".to_owned(),
        },
    });
    let mut bytes = request.encode();
    assert_eq!(bytes.len(), super::MAX_VOLUME_FRAME);
    assert_eq!(Request::decode(&bytes), Ok(request));
    bytes.pop();
    assert!(Request::decode(&bytes).is_err());
}
