//! Workspace handoff on the owning host, using only the volume store's certified description.

use std::{num::NonZeroU64, path::PathBuf};

use soma::{BackendFailureKind, InstanceId};
use soma_guest::WorkspaceOutcome;
use soma_hostd::{Reply, Request, client::HostClient};
use soma_vmm::sandbox::WorkspaceChange;

use crate::backend::kvm::{KvmBackend, jailed_volume::open_locked, runtime::SOCKET_VARIABLE};

impl KvmBackend {
    pub(in crate::backend::kvm) fn change_resident_workspace(
        &mut self,
        instance: &InstanceId,
        volume_id: [u8; 16],
        attach: bool,
    ) -> Result<(), BackendFailureKind> {
        let rejected = BackendFailureKind::WorkloadRejected;
        let unavailable = BackendFailureKind::Unavailable;
        let host_instance =
            soma_hostd::InstanceId::from_hex(instance.as_str()).map_err(|_| rejected)?;
        let live = self.running_for(instance)?;
        if !live.checkpoint_source.has_volume {
            return Err(BackendFailureKind::Unsupported);
        }
        let socket = PathBuf::from(std::env::var_os(SOCKET_VARIABLE).ok_or(unavailable)?);
        let host = HostClient::connect(&socket).map_err(|_| unavailable)?;
        let Reply::VolumeOwned { volume, .. } = host
            .call(&Request::VolumeOwned {
                instance: host_instance,
            })
            .map_err(|_| unavailable)?
        else {
            return Err(rejected);
        };
        if volume.as_bytes() != &volume_id {
            return Err(rejected);
        }
        if attach != live.workspace_offline {
            return Ok(());
        }
        let change = if attach {
            let host = HostClient::connect(&socket).map_err(|_| unavailable)?;
            let Reply::VolumeOpened {
                volume,
                size_class_gib,
                path,
                ..
            } = host
                .call(&Request::VolumeOpen {
                    instance: host_instance,
                })
                .map_err(|_| unavailable)?
            else {
                return Err(unavailable);
            };
            if volume.as_bytes() != &volume_id {
                return Err(rejected);
            }
            let capacity = NonZeroU64::new(u64::from(size_class_gib) << 30).ok_or(rejected)?;
            WorkspaceChange::Attach(open_locked(&PathBuf::from(path), capacity)?)
        } else {
            WorkspaceChange::Detach
        };
        match live.held.change_workspace(change)? {
            WorkspaceOutcome::Ready => {
                live.workspace_offline = !attach;
                Ok(())
            }
            WorkspaceOutcome::Refused => Err(BackendFailureKind::ResourceConflict),
            WorkspaceOutcome::Unsupported => Err(BackendFailureKind::Unsupported),
            WorkspaceOutcome::Inconsistent => Err(BackendFailureKind::GuestFailure),
        }
    }
}
