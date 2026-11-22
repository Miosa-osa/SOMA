use std::{os::fd::AsFd as _, time::Duration};

use soma::BackendFailureKind;
use soma_guest::WorkspaceOutcome;
use soma_vmm::{
    control::{Outcome, Request},
    sandbox::WorkspaceChange,
};

use super::Jailed;

impl Jailed {
    pub(in crate::backend::kvm) fn change_workspace(
        &mut self,
        change: WorkspaceChange,
    ) -> Result<WorkspaceOutcome, BackendFailureKind> {
        if !self.is_usable() {
            return Err(BackendFailureKind::Unavailable);
        }
        let within = Duration::from_secs(95);
        let reply = match change {
            WorkspaceChange::Detach => self.control.ask(
                &Request::Workspace {
                    instance: self.instance,
                    attach: false,
                },
                within,
            ),
            WorkspaceChange::Attach(file) => self.control.ask_with_files(
                &Request::Workspace {
                    instance: self.instance,
                    attach: true,
                },
                &[file.as_fd()],
                within,
            ),
        };
        match reply {
            Ok(Outcome::Workspace(instance, outcome)) if instance == self.instance => Ok(outcome),
            Ok(Outcome::Rejected(_)) => Ok(WorkspaceOutcome::Unsupported),
            _ => Err(self.poison(BackendFailureKind::GuestFailure)),
        }
    }
}
