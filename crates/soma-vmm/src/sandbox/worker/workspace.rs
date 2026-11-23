//! Coordinates the guest filesystem and the resident virtio slot under one worker.

use std::fs::File;
use std::time::Duration;

use soma_guest::{GuestApi, RepairedHostControl, WorkspaceAction, WorkspaceOutcome};
use soma_kvm::x86_64::SandboxMachine;

use super::super::{
    CheckpointJob, CheckpointJobError, SessionError, WorkspaceChange, identity::fresh16,
    io::HostIo, session::Response,
};
use super::files::nonce_text;

type Control<'a> = RepairedHostControl<HostIo<'a>>;
type Exchange<'a> = Result<(Control<'a>, WorkspaceOutcome), SessionError>;

#[derive(Default)]
pub(super) struct Workspace {
    offline: bool,
}

impl Workspace {
    pub(super) const fn is_offline(&self) -> bool {
        self.offline
    }

    pub(super) fn checkpoint(&self, job: &CheckpointJob, machine: &SandboxMachine) -> Response {
        let outcome = if self.is_offline() {
            Err(CheckpointJobError::CaptureRefused)
        } else {
            job.capture(machine).map(Box::new)
        };
        Response::Checkpointed(outcome)
    }

    pub(super) fn change<'a>(
        &mut self,
        machine: &SandboxMachine,
        control: Control<'a>,
        change: WorkspaceChange,
        api: &mut Option<GuestApi>,
    ) -> Exchange<'a> {
        let (control, level) = control
            .ensure_guest_api(api, || nonce_text(&fresh16()).into_bytes())
            .map_err(|_| SessionError::Workspace)?;
        if !level.has_workspace_mounts() {
            return Ok((control, WorkspaceOutcome::Unsupported));
        }
        match change {
            WorkspaceChange::Detach if self.offline => Ok((control, WorkspaceOutcome::Ready)),
            WorkspaceChange::Attach(_) if !self.offline => Ok((control, WorkspaceOutcome::Refused)),
            WorkspaceChange::Detach => self.detach(machine, control),
            WorkspaceChange::Attach(file) => self.attach(machine, control, file),
        }
    }

    fn detach<'a>(&mut self, machine: &SandboxMachine, control: Control<'a>) -> Exchange<'a> {
        let (control, outcome) = request(control, WorkspaceAction::Unmount)?;
        if outcome != WorkspaceOutcome::Ready {
            return Ok((control, outcome));
        }
        if set_backing(machine, None)? {
            self.offline = true;
            return Ok((control, WorkspaceOutcome::Ready));
        }
        let (control, rollback) = request(control, WorkspaceAction::Mount)?;
        if rollback != WorkspaceOutcome::Ready {
            return Err(SessionError::Workspace);
        }
        Ok((control, WorkspaceOutcome::Refused))
    }

    fn attach<'a>(
        &mut self,
        machine: &SandboxMachine,
        control: Control<'a>,
        file: File,
    ) -> Exchange<'a> {
        if !set_backing(machine, Some(file))? {
            return Ok((control, WorkspaceOutcome::Refused));
        }
        let (control, outcome) = request(control, WorkspaceAction::Mount)?;
        if outcome == WorkspaceOutcome::Ready {
            self.offline = false;
            return Ok((control, outcome));
        }
        if outcome != WorkspaceOutcome::Refused || !set_backing(machine, None)? {
            return Err(SessionError::Workspace);
        }
        Ok((control, outcome))
    }
}

fn request(control: Control<'_>, action: WorkspaceAction) -> Exchange<'_> {
    let (control, outcome) = control
        .workspace(action)
        .map_err(|_| SessionError::Workspace)?;
    if outcome == WorkspaceOutcome::Inconsistent {
        return Err(SessionError::Workspace);
    }
    Ok((control, outcome))
}

fn set_backing(machine: &SandboxMachine, backing: Option<File>) -> Result<bool, SessionError> {
    machine
        .pause(Duration::from_secs(2))
        .map_err(|_| SessionError::Pause)?;
    let swapped = match machine.set_paused_workspace(backing) {
        Ok(()) => true,
        Err(error) => {
            eprintln!("soma-vmm: workspace backing change refused: {error}");
            false
        }
    };
    machine.resume().map_err(|_| SessionError::Resume)?;
    Ok(swapped)
}
