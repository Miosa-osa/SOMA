use std::thread;

use crate::{
    FileOutcome, GuestControl, GuestRequest, HostControl, WorkspaceAction, WorkspaceOutcome,
};

use super::support::{deadline, launch, pair};

#[test]
fn old_guest_refuses_workspace_mounts_without_losing_control() {
    roundtrip(5);
}

#[test]
fn level_six_guest_reports_distinct_mount_outcomes_over_the_same_session() {
    roundtrip(6);
}

fn roundtrip(level: u8) {
    let (host_material, guest_material) = launch();
    let (host_io, guest_io, host_observed, guest_observed) = pair();
    let guest_thread = thread::spawn(move || {
        let guest =
            GuestControl::connect(guest_material, guest_io, deadline()).expect("guest connect");
        let (guest, _) = guest.next_request(deadline()).expect("prepare");
        let guest = guest.repair_complete(deadline()).expect("repair");
        let (guest, probe) = guest.next_request(deadline()).expect("level probe");
        assert!(matches!(probe, GuestRequest::File { .. }));
        let guest = guest
            .file_outcome(
                &FileOutcome::Read {
                    bytes: format!("{level} nonce\n").into_bytes().into(),
                    end: true,
                },
                deadline(),
            )
            .expect("probe outcome");
        let mut guest = guest;
        if level == 6 {
            for (expected, outcome) in [
                (WorkspaceAction::Unmount, WorkspaceOutcome::Refused),
                (WorkspaceAction::Mount, WorkspaceOutcome::Ready),
            ] {
                let (next, request) = guest.next_request(deadline()).expect("workspace request");
                assert!(
                    matches!(request, GuestRequest::Workspace { action, .. } if action == expected)
                );
                guest = next
                    .workspace_outcome(outcome, deadline())
                    .expect("workspace answer");
            }
        }
        let (guest, request) = guest.next_request(deadline()).expect("shutdown");
        assert!(matches!(request, GuestRequest::Shutdown { .. }));
        guest.shutdown_ack(deadline()).expect("shutdown answer");
    });
    let host = HostControl::connect(host_material, host_io).expect("host connect");
    let host = host.prepare().expect("ready host");
    let (host, _) = host.probe_guest_api(b"nonce").expect("capability");
    let (host, outcome) = host.workspace(WorkspaceAction::Unmount).expect("unmount");
    let host = if level == 6 {
        assert_eq!(outcome, WorkspaceOutcome::Refused);
        let (host, outcome) = host.workspace(WorkspaceAction::Mount).expect("mount");
        assert_eq!(outcome, WorkspaceOutcome::Ready);
        host
    } else {
        assert_eq!(outcome, WorkspaceOutcome::Unsupported);
        host
    };
    host.shutdown(super::host_file::operation(9))
        .expect("shutdown");
    guest_thread.join().expect("guest thread");
    assert_eq!(host_observed.poison(), 0);
    assert_eq!(guest_observed.poison(), 0);
}
