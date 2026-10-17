//! Building one machine host: the process, and for a primed one its restored identity-free VM.

use std::{
    io::BufReader,
    os::fd::{AsFd as _, OwnedFd},
    os::unix::net::UnixStream,
    process::{Child, Command, Stdio},
};

use soma::BackendFailureKind;

use super::{PrewarmPlan, SterileHost};
use crate::backend::kvm::host::{
    channel,
    wire::{InitialWire, PrewarmReady, PrewarmWire},
};

pub(super) fn spawn(plan: Option<&PrewarmPlan>) -> Result<SterileHost, BackendFailureKind> {
    spawn_explained(plan).map_err(|failure| failure.kind)
}

/// Why a machine host could not be built: the typed kind a launch maps, and the step that failed.
pub(super) struct SpawnFailure {
    pub(super) kind: BackendFailureKind,
    pub(super) reason: String,
}

impl SpawnFailure {
    pub(super) fn at(step: &str) -> Self {
        Self {
            kind: BackendFailureKind::Unavailable,
            reason: step.to_owned(),
        }
    }
}

pub(super) fn spawn_explained(plan: Option<&PrewarmPlan>) -> Result<SterileHost, SpawnFailure> {
    let executable = std::env::current_exe()
        .map_err(|_| SpawnFailure::at("the running executable is unknown"))?;
    let (handoff, child_input) =
        UnixStream::pair().map_err(|_| SpawnFailure::at("no socket pair for the machine host"))?;
    let child = Command::new(executable)
        .arg("machine-host")
        .stdin(Stdio::from(OwnedFd::from(child_input)))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| SpawnFailure::at("the machine host process could not be started"))?;
    let mut host = SterileHost {
        child,
        handoff,
        output: None,
        primed: None,
    };
    if let Some(plan) = plan
        && let Err(failure) = prime(&mut host, plan)
    {
        terminate(&mut host.child);
        return Err(failure);
    }
    Ok(host)
}

pub(super) fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn prime(host: &mut SterileHost, plan: &PrewarmPlan) -> Result<(), SpawnFailure> {
    let (manifest, artifacts) = plan
        .prepared
        .handoff()
        .map_err(|_| SpawnFailure::at("the prepared Generation could not be opened for handoff"))?;
    let borrowed = artifacts
        .iter()
        .map(std::fs::File::as_fd)
        .collect::<Vec<_>>();
    soma_supervise::send_descriptors(host.handoff.as_fd(), &borrowed).map_err(|_| {
        SpawnFailure::at("the machine host did not accept the artifact descriptors")
    })?;
    channel::write_line(
        &mut host.handoff,
        &InitialWire::Prewarm(PrewarmWire {
            // Read here rather than captured when the target was installed, because the set is
            // learned from launches that happened after it was.
            warm: super::warm_for(&plan.key())
                .into_iter()
                .map(super::super::wire::HeadSpanWire::from)
                .collect(),
            reference: plan.prepared.reference.clone(),
            generation_id: plan.prepared.id.clone(),
            manifest,
            memory_mib: plan.memory_mib,
        }),
    )
    .map_err(|()| SpawnFailure::at("the machine host did not accept the prewarm request"))?;
    let output = host
        .child
        .stdout
        .take()
        .ok_or_else(|| SpawnFailure::at("the machine host has no output channel"))?;
    let mut output = BufReader::new(output);
    match channel::read_line(&mut output) {
        Some(PrewarmReady::Prepared) => {}
        Some(PrewarmReady::Refused) => {
            return Err(SpawnFailure::at(
                "the machine host refused to restore this Generation at this memory",
            ));
        }
        None => {
            // No answer: say whether the process is gone, and how, because that is the evidence.
            let reason = match host.child.try_wait() {
                Ok(Some(status)) => format!("the machine host exited before answering ({status})"),
                _ => "the machine host closed its channel without answering".to_owned(),
            };
            return Err(SpawnFailure::at(&reason));
        }
    }
    host.output = Some(output);
    host.primed = Some(plan.key());
    Ok(())
}
