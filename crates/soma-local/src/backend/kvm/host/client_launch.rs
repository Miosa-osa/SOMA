//! Parent-side claim and launch handshake for one resident machine host.

use std::{
    io::BufReader,
    os::fd::AsFd as _,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Child,
};

use soma::{BackendFailureKind, InstanceId, MachineShape, OperationId};

use super::{
    channel, reaper, sterile,
    wire::{InitialWire, LaunchWire, Launched, Ready},
};
use crate::backend::kvm::prepared::PreparedGeneration;

struct LaunchContext<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    socket: PathBuf,
}

/// Claims the host that will hold one machine, and returns what its launch established.
pub(in crate::backend::kvm) fn launch(
    directory: &Path,
    operation_id: &OperationId,
    instance_id: &InstanceId,
    prepared: &PreparedGeneration,
    shape: &MachineShape,
) -> Result<Launched, BackendFailureKind> {
    if !channel::addressable(directory) {
        return Err(BackendFailureKind::Unsupported);
    }
    channel::prepare_directory(directory).map_err(|()| BackendFailureKind::Unavailable)?;
    let socket = channel::socket_path(directory, instance_id);
    let sterile::SterileHost {
        mut child,
        handoff,
        output,
        ..
    } = sterile::claim(prepared, shape.memory_mib())?;
    let context = LaunchContext {
        operation_id,
        instance_id,
        socket,
    };
    match handshake(&mut child, handoff, output, context, prepared, shape) {
        Ok(launched) => {
            reaper::adopt(child).map_err(|_| BackendFailureKind::Unavailable)?;
            Ok(launched)
        }
        Err(kind) => {
            let _ignored = child.kill();
            let _ignored = child.wait();
            Err(kind)
        }
    }
}

fn handshake(
    child: &mut Child,
    mut handoff: UnixStream,
    output: Option<BufReader<std::process::ChildStdout>>,
    context: LaunchContext<'_>,
    prepared: &PreparedGeneration,
    shape: &MachineShape,
) -> Result<Launched, BackendFailureKind> {
    let request = LaunchWire {
        socket: context.socket,
        operation_id: context.operation_id.clone(),
        instance_id: context.instance_id.clone(),
        reference: prepared.reference.clone(),
        generation_id: prepared.id.clone(),
        manifest: soma_generation::generation_manifest::encode_manifest(&prepared.manifest)
            .map_err(|_| BackendFailureKind::Unavailable)?,
        shape: shape.clone(),
    };
    if output.is_none() {
        let (_manifest, artifacts) = prepared
            .handoff()
            .map_err(|_| BackendFailureKind::Unavailable)?;
        let borrowed = artifacts
            .iter()
            .map(std::fs::File::as_fd)
            .collect::<Vec<_>>();
        soma_supervise::send_descriptors(handoff.as_fd(), &borrowed)
            .map_err(|_| BackendFailureKind::Unavailable)?;
        channel::write_line(&mut handoff, &InitialWire::Launch(Box::new(request)))
            .map_err(|()| BackendFailureKind::Unavailable)?;
    } else {
        channel::write_line(&mut handoff, &request)
            .map_err(|()| BackendFailureKind::Unavailable)?;
    }
    drop(handoff);
    let mut reader = match output {
        Some(output) => output,
        None => BufReader::new(child.stdout.take().ok_or(BackendFailureKind::Unavailable)?),
    };
    match channel::read_line::<Ready>(&mut reader) {
        Some(Ready::Launched(launched)) => Ok(launched),
        Some(Ready::Refused(refusal)) => Err(refusal.into()),
        None => Err(BackendFailureKind::Unavailable),
    }
}
