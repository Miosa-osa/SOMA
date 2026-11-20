//! Parent-side claim and launch handshake for one resident machine host.

use std::{
    io::BufReader,
    os::fd::AsFd as _,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Child,
    time::Instant,
};

use soma::{BackendFailureKind, InstanceId, MachineShape, OperationId};

use super::{
    channel, reaper, sterile, warm_gate,
    wire::{InitialWire, LaunchWire, Launched, Ready},
};
use crate::backend::kvm::prepared::PreparedGeneration;

struct LaunchContext<'a> {
    operation_id: &'a OperationId,
    instance_id: &'a InstanceId,
    socket: PathBuf,
    skip_warm: bool,
    volume: Option<&'a soma::VolumeAttachment>,
    windowed: bool,
    /// Carried from [`HostedLaunch`] to the handoff, which is the only place that reads it.
    admitted_at: Instant,
}

/// One launch a resident machine host is asked to perform, as the process that admitted it holds
/// it.
#[derive(Clone, Copy)]
pub(in crate::backend::kvm) struct HostedLaunch<'a> {
    /// Where hosted machines are addressed.
    pub(in crate::backend::kvm) directory: &'a Path,
    pub(in crate::backend::kvm) operation_id: &'a OperationId,
    pub(in crate::backend::kvm) instance_id: &'a InstanceId,
    /// The Generation this launch serves, re-verified before a machine is claimed.
    pub(in crate::backend::kvm) prepared: &'a PreparedGeneration,
    pub(in crate::backend::kvm) shape: &'a MachineShape,
    pub(in crate::backend::kvm) volume: Option<&'a soma::VolumeAttachment>,
    /// Whether the machine must hold a network whose egress can be opened after launch.
    pub(in crate::backend::kvm) windowed: bool,
    /// When this operation was admitted, on the admitting process's clock.
    ///
    /// The machine is built in another process, whose `Instant`s mean nothing here, so this is
    /// what places the child's own milestones on the timeline the caller reads. See
    /// [`handshake`].
    pub(in crate::backend::kvm) admitted_at: Instant,
}

/// Claims the host that will hold one machine, and returns what its launch established.
pub(in crate::backend::kvm) fn launch(
    request: HostedLaunch<'_>,
) -> Result<Launched, BackendFailureKind> {
    let HostedLaunch {
        directory,
        operation_id,
        instance_id,
        prepared,
        shape,
        volume,
        windowed,
        admitted_at,
    } = request;
    if !channel::addressable(directory) {
        return Err(BackendFailureKind::Unsupported);
    }
    // A request the Generation can never give is refused before a prepared machine is claimed, so
    // it neither consumes a machine nor counts as demand for the shape.
    if !crate::backend::kvm::fit::request_fits(prepared, shape) {
        return Err(BackendFailureKind::WorkloadRejected);
    }
    // Counted from the claim to the answer, which is the stretch in which a burst's launches
    // overlap; the warm command itself runs in the host after the answer.
    let in_flight = warm_gate::enter();
    channel::prepare_directory(directory).map_err(|()| BackendFailureKind::Unavailable)?;
    let socket = channel::socket_path(directory, instance_id);
    let sterile::SterileHost {
        mut child,
        handoff,
        output,
        ..
    } = sterile::claim(
        prepared,
        prepared.manifest.shape.memory_bytes / (1024 * 1024),
    )?;
    let context = LaunchContext {
        operation_id,
        instance_id,
        socket,
        skip_warm: in_flight.skip_warm(),
        volume,
        windowed,
        admitted_at,
    };
    match handshake(&mut child, handoff, output, context, prepared, shape) {
        // The host's own answer must show the gate was kept. A host that served the launch
        // without it (an older build that dropped the field, or a claim that fell back to the
        // bundle-free machine) answers with a detached network, and that machine is torn down
        // here rather than handed back as one whose egress a window could change.
        Ok(launched) if !honours_windowed(windowed, launched.network.attachment()) => {
            let _ignored = child.kill();
            let _ignored = child.wait();
            Err(BackendFailureKind::Unsupported)
        }
        Ok(launched) => {
            // What the guest read from its head travels back with the answer, so the pool can warm
            // the next head for this Generation while it is prepared rather than while the next
            // guest waits. A launch that read nothing leaves the set as it was.
            let spans: Vec<soma_kvm::head_reads::HeadSpan> = launched
                .head_reads
                .iter()
                .copied()
                .map(Into::into)
                .collect();
            sterile::note_head_reads(&sterile::key_for(prepared), &spans);
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

/// Whether a launch's observed network is the one a windowed request needs: attached to a
/// bundle. A request that did not ask for a window is satisfied by anything.
pub(super) fn honours_windowed(
    windowed: bool,
    attachment: &soma::Observation<soma::NetworkAttachment>,
) -> bool {
    !windowed
        || matches!(
            attachment,
            soma::Observation::Observed(soma::NetworkAttachment::Attached)
        )
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
        // The volume's host path, certified by the store before the launch was admitted; the
        // file itself never crosses this wire.
        volume: context.volume.cloned(),
        operation_id: context.operation_id.clone(),
        instance_id: context.instance_id.clone(),
        reference: prepared.reference.clone(),
        generation_id: prepared.id.clone(),
        manifest: soma_generation::generation_manifest::encode_manifest(&prepared.manifest)
            .map_err(|_| BackendFailureKind::Unavailable)?,
        shape: shape.clone(),
        skip_warm: context.skip_warm,
        windowed: context.windowed,
    };
    // Taken as late as it can be, because time before this point is time this launch spent
    // reaching the process that will build it, and a child can only time what happens after the
    // launch arrives. Only the socket's own carry is left uncounted, which between two processes
    // on one host is microseconds.
    let handoff_ns: u64;
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
        handoff_ns = since_ns(context.admitted_at);
        channel::write_line(&mut handoff, &InitialWire::Launch(Box::new(request)))
            .map_err(|()| BackendFailureKind::Unavailable)?;
    } else {
        handoff_ns = since_ns(context.admitted_at);
        channel::write_line(&mut handoff, &request)
            .map_err(|()| BackendFailureKind::Unavailable)?;
    }
    drop(handoff);
    let mut reader = match output {
        Some(output) => output,
        None => BufReader::new(child.stdout.take().ok_or(BackendFailureKind::Unavailable)?),
    };
    match channel::read_line::<Ready>(&mut reader) {
        Some(Ready::Launched(mut launched)) => {
            report_unsurvivable(&launched);
            // The child timed its own construction from the moment it received the launch. Adding
            // the two elapsed times is what makes `machine_launched` mean "the machine existed"
            // rather than "the machine was asked to exist": without it the child's clock starts at
            // its own launch stamp, the stamp reports zero, and every second the launch spent
            // being built is charged to the readiness that follows it.
            launched.at_ns = launched.at_ns.saturating_add(handoff_ns);
            Ok(launched)
        }
        Some(Ready::Refused(refusal)) => Err(refusal.into()),
        None => Err(BackendFailureKind::Unavailable),
    }
}

/// Nanoseconds from `at` to now, saturating rather than wrapping on a clock that never runs back.
fn since_ns(at: Instant) -> u64 {
    u64::try_from(at.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

/// Says so when a claimed host could not leave the unit's control group, since the host has no log.
fn report_unsurvivable(launched: &Launched) {
    match launched.live_cgroup {
        Some(report) if !report.joined => eprintln!(
            "soma-api: a machine host could not leave the unit's control group; that sandbox will not survive a restart of the API"
        ),
        Some(report) if !report.accounted => eprintln!(
            "soma-api: a machine host left the unit's control group but the memory or pids controller is not enabled in the live subtree, so it is not accounted"
        ),
        _ => {}
    }
}
