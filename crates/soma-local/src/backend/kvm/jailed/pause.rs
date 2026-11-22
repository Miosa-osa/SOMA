use std::time::Duration;

use soma::BackendFailureKind;
use soma_vmm::control::{Outcome, Request};

use super::Jailed;

impl Jailed {
    pub(in crate::backend::kvm) fn pause(&mut self) -> Result<(), BackendFailureKind> {
        if !self.is_usable() {
            return Err(BackendFailureKind::Unavailable);
        }
        match self
            .control
            .ask(&Request::Pause(self.instance), Duration::from_secs(75))
        {
            Ok(Outcome::Paused(instance)) if instance == self.instance => Ok(()),
            Ok(Outcome::Rejected(_)) => Err(BackendFailureKind::Unsupported),
            _ => Err(self.poison(BackendFailureKind::GuestFailure)),
        }
    }
    pub(in crate::backend::kvm) fn resume(&mut self) -> Result<(), BackendFailureKind> {
        if !self.is_usable() {
            return Err(BackendFailureKind::Unavailable);
        }
        match self
            .control
            .ask(&Request::Resume(self.instance), Duration::from_secs(75))
        {
            Ok(Outcome::Resumed(instance)) if instance == self.instance => Ok(()),
            Ok(Outcome::Rejected(_)) => Err(BackendFailureKind::Unsupported),
            _ => Err(self.poison(BackendFailureKind::GuestFailure)),
        }
    }
}
