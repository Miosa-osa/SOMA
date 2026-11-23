//! Serializes the volume store and live worker changes through the sole owner of the Instance.

use std::{num::NonZeroU64, path::PathBuf};

use soma::{BackendFailureKind, InstanceId};
use soma_guest::WorkspaceOutcome;
use soma_hostd::{AttachFrame, DetachFrame, Reply, Request, client::HostClient};
use soma_vmm::sandbox::WorkspaceChange;

use crate::backend::kvm::{KvmBackend, jailed_volume::open_locked, runtime::SOCKET_VARIABLE};

use super::Live;

type Transition = Result<Reply, BackendFailureKind>;

impl KvmBackend {
    pub(in crate::backend::kvm) fn change_resident_workspace(
        &mut self,
        instance: &InstanceId,
        request: Request,
    ) -> Transition {
        let rejected = BackendFailureKind::WorkloadRejected;
        let host_instance =
            soma_hostd::InstanceId::from_hex(instance.as_str()).map_err(|_| rejected)?;
        let live = self.running_for(instance)?;
        if !live.checkpoint_source.has_volume {
            return Err(BackendFailureKind::Unsupported);
        }
        let socket = PathBuf::from(
            std::env::var_os(SOCKET_VARIABLE).ok_or(BackendFailureKind::Unavailable)?,
        );
        match request {
            Request::VolumeAttach(frame) if frame.instance == host_instance => {
                let generation = soma_hostd::GenerationId::from_hex(
                    live.generation.as_str().trim_start_matches("sha256:"),
                )
                .map_err(|_| rejected)?;
                if frame.generation != generation {
                    return Err(rejected);
                }
                attach(live, &socket, frame)
            }
            Request::VolumeDetach(frame) if frame.instance == host_instance => {
                detach(live, &socket, frame)
            }
            _ => Err(rejected),
        }
    }
}

fn attach(live: &mut Live, socket: &std::path::Path, frame: AttachFrame) -> Transition {
    if live.workspace_offline {
        let binding = call(socket, &Request::WorkspaceStage(frame.clone()))?;
        let Reply::VolumeOpened {
            volume,
            size_class_gib,
            path,
            ..
        } = binding
        else {
            return fail_or_unavailable(binding);
        };
        if volume != frame.volume || size_class_gib != frame.size_class_gib {
            return Err(BackendFailureKind::WorkloadRejected);
        }
        let capacity = NonZeroU64::new(u64::from(size_class_gib) << 30)
            .ok_or(BackendFailureKind::WorkloadRejected)?;
        let file = open_locked(&PathBuf::from(path), capacity)?;
        confirm_worker(live, WorkspaceChange::Attach(file))?;
        live.workspace_offline = false;
    }
    call(socket, &Request::WorkspaceCommit(frame))
}

fn detach(live: &mut Live, socket: &std::path::Path, frame: DetachFrame) -> Transition {
    let admitted = call(socket, &Request::WorkspaceAdmitDetach(frame))?;
    if !matches!(admitted, Reply::VolumeOwned { volume, .. } if volume == frame.volume) {
        return fail_or_unavailable(admitted);
    }
    if !live.workspace_offline {
        confirm_worker(live, WorkspaceChange::Detach)?;
        live.workspace_offline = true;
    }
    call(socket, &Request::VolumeDetach(frame))
}

fn confirm_worker(live: &mut Live, change: WorkspaceChange) -> Result<(), BackendFailureKind> {
    match live.held.change_workspace(change)? {
        WorkspaceOutcome::Ready => Ok(()),
        WorkspaceOutcome::Refused => Err(BackendFailureKind::ResourceConflict),
        WorkspaceOutcome::Unsupported => Err(BackendFailureKind::Unsupported),
        WorkspaceOutcome::Inconsistent => Err(BackendFailureKind::GuestFailure),
    }
}

fn call(socket: &std::path::Path, request: &Request) -> Transition {
    HostClient::connect(socket)
        .and_then(|host| host.call(request))
        .map_err(|_| BackendFailureKind::Unavailable)
}

fn fail_or_unavailable(reply: Reply) -> Transition {
    if matches!(reply, Reply::Failed(_)) {
        Ok(reply)
    } else {
        Err(BackendFailureKind::Unavailable)
    }
}
