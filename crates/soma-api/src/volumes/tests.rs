use soma_hostd::FailureCode;

use super::refusal::refusal_for;
use super::*;

fn hex(fill: u8, len: usize) -> String {
    std::iter::repeat_n(format!("{fill:02x}"), len).collect()
}

fn body() -> VolumeBody {
    VolumeBody {
        operation_id: hex(0xa1, 16),
        instance_id: hex(0xb2, 16),
        volume_id: hex(0xc3, 16),
        request_fingerprint: hex(0xd4, 32),
        size_class_gib: 10,
        generation_id: Some(hex(0xe5, 32)),
        tenant: Some("tenant-alpha".to_owned()),
    }
}

fn failure_code_of(code: FailureCode) -> u16 {
    soma_hostd::failure_code(code)
}

#[test]
fn an_unconfigured_client_reports_the_missing_capability() {
    let client = VolumeClient::at(None);
    assert!(!client.configured());
    assert_eq!(
        client.call(VolumeOperation::Read, &body()).unwrap_err(),
        VolumeFailure::Unconfigured
    );
    assert_eq!(VolumeFailure::Unconfigured.refusal().status(), 501);
}

#[test]
fn the_environment_selects_the_socket_and_a_blank_value_is_unset() {
    assert!(socket_from(Some("/run/soma-hostd/allocator.sock".into())).is_some());
    assert!(socket_from(Some("   ".into())).is_some());
    assert!(socket_from(Some("".into())).is_none());
    assert!(socket_from(None).is_none());
    assert!(VolumeClient::at(Some("/sock".into())).configured());
}

#[test]
fn every_operation_builds_the_frame_the_daemon_reads() {
    let body = body();
    assert!(matches!(
        request_for(VolumeOperation::Attach, &body),
        Ok(Request::VolumeAttach(_))
    ));
    assert!(matches!(
        request_for(VolumeOperation::Detach, &body),
        Ok(Request::VolumeDetach(_))
    ));
    assert!(matches!(
        request_for(VolumeOperation::Destroy, &body),
        Ok(Request::VolumeDestroy(_))
    ));
    assert!(matches!(
        request_for(VolumeOperation::Read, &body),
        Ok(Request::VolumeRead { .. })
    ));
}

#[test]
fn a_frame_whose_fields_do_not_decode_is_malformed() {
    let mut broken = body();
    broken.volume_id = "not-hex".to_owned();
    assert_eq!(
        request_for(VolumeOperation::Detach, &broken).unwrap_err(),
        VolumeFailure::Malformed
    );
    let mut missing = body();
    missing.generation_id = None;
    assert_eq!(
        request_for(VolumeOperation::Attach, &missing).unwrap_err(),
        VolumeFailure::Malformed
    );
}

#[test]
fn every_stable_daemon_code_maps_to_the_status_the_interface_fixes() {
    let cases = [
        (FailureCode::VolumeMissing, 404),
        (FailureCode::VolumeBusy, 409),
        (FailureCode::Conflict, 409),
        (FailureCode::VolumeFsckFailed, 409),
        (FailureCode::VolumeRepairRefused, 409),
        (FailureCode::VolumeEncryption, 409),
        (FailureCode::VolumeGeometry, 409),
        (FailureCode::Exhausted, 507),
        (FailureCode::Capacity, 507),
        (FailureCode::Ledger, 500),
    ];
    for (code, status) in cases {
        assert_eq!(refusal_for(failure_code_of(code)).status(), status);
    }
    // A code this build does not know is a refusal, never an invented success.
    assert_eq!(refusal_for(9999).status(), 500);
}

#[test]
fn a_reply_renders_the_report_the_control_plane_reads() {
    let body = body();
    let attached = Reply::VolumeAttached {
        volume: HostVolume::from_hex(&body.volume_id).expect("hex"),
        state: 1,
        repair: 2,
        exit_code: 0,
    };
    let report = report_for(VolumeOperation::Attach, &body, &attached).expect("renders");
    assert_eq!(report.state, "attached");
    assert_eq!(report.device, VOLUME_DEVICE);
    assert!(report.ready);
    let fsck = report.fsck.expect("a receipt");
    assert!(fsck.repaired);
    assert_eq!(fsck.tool, "e2fsck");

    let refused = Reply::Failed(failure_code_of(FailureCode::VolumeBusy));
    assert_eq!(
        report_for(VolumeOperation::Detach, &body, &refused).unwrap_err(),
        VolumeFailure::Refused(15)
    );
}
