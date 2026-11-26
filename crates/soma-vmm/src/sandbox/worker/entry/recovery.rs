//! Restore a checkpoint with a fresh authenticated guest session.

use std::sync::mpsc::{Receiver, Sender};

use soma_guest::HostLaunchMaterial;
use soma_kvm::snapshot::compatibility::Incompatibility;
use soma_kvm::x86_64::{NetworkAttachment, SnapshotError, prepare_checkpoint, prepare_fork};

use super::{
    Boot, PendingActivation, Request, Response, SessionError, Source, drive_checkpoint, report,
};

pub(super) fn matches_recovery_source(boot: &Boot) -> bool {
    let (request, names_expected_instance, admit_network) = match &boot.source {
        Source::ResidentCheckpoint(request) => (
            request,
            request.instance_id == boot.instance,
            request.devices.net(),
        ),
        // A fork takes a network of its own: the attachment is the child's fresh claim and its
        // MAC must be the one on the child's launch page, never the source's.
        Source::CheckpointFork(request) => (
            request,
            request.instance_id != boot.instance,
            request.devices.net(),
        ),
        _ => return true,
    };
    let network_valid = match (&boot.network.attachment, &boot.network.activation) {
        (None, None) => true,
        (Some(attachment), Some(_)) => admit_network && attachment.mac == boot.network.launch.mac(),
        _ => false,
    };
    network_valid
        && boot.secrets.is_empty()
        && names_expected_instance
        && request.candidate_id == boot.generation
        && request.guest_cid == boot.guest_cid
        && boot.network.launch.vsock_cid() == boot.guest_cid
}

pub(super) fn recover(
    request: soma_kvm::x86_64::ResidentRestoreRequest,
    material: HostLaunchMaterial,
    identity: ([u8; 16], [u8; 16]),
    network: (Option<NetworkAttachment>, Option<PendingActivation>),
    requests: &Receiver<Request>,
    responses: &Sender<Response>,
) {
    let (instance, operation) = identity;
    let source = request.instance_id;
    let (prepared, material) = if source == instance {
        (prepare_checkpoint(request), Ok(material))
    } else {
        let authority = material.for_fork(source);
        (prepare_fork(request, instance), authority)
    };
    let Ok(material) = material else {
        let _ignored = responses.send(Response::Failed(SessionError::LaunchPage));
        return;
    };
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            eprintln!("soma-vmm: checkpoint validation failed: {error}");
            let refusal = match error {
                SnapshotError::Incompatible(Incompatibility::CpuTemplate { .. }) => {
                    SessionError::CheckpointIncompatible
                }
                _ => SessionError::Create,
            };
            let _ignored = responses.send(Response::Failed(refusal));
            return;
        }
    };
    let (attachment, activation) = network;
    if let Some(attachment) = attachment {
        prepared.attach_network(attachment);
    }
    let mut restored = None;
    let delivered = material.deliver_with(|page| {
        restored = Some(prepared.resume(page)?);
        Ok::<(), soma_kvm::x86_64::SnapshotError>(())
    });
    let (Ok(delivered), Some(restored)) = (delivered, restored) else {
        let _ignored = responses.send(Response::Failed(SessionError::LaunchPage));
        return;
    };
    let outcome = drive_checkpoint(
        &restored,
        delivered,
        (instance, operation, activation),
        requests,
        responses,
    );
    report(restored.machine, &outcome, responses, instance);
}
