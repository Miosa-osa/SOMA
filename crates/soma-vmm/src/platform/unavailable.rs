use crate::{Execute, ExitStatus, Launch, Recovery, Stop};

use super::{
    Platform, PlatformExecution, PlatformFailure, PlatformStop, ReadinessFailure,
    ReadinessProgress, ReadyAuthenticatedGuest, RestoreFailure, RestoreProgress, RestoredMachine,
};

pub(crate) struct UnavailablePlatform;

impl Platform for UnavailablePlatform {
    fn verify_and_restore(&mut self, _launch: &Launch) -> Result<RestoredMachine, RestoreFailure> {
        RestoredMachine::from_observation(RestoreProgress::from_steps([]))
    }

    fn authenticate_repair_and_ready(
        &mut self,
        _launch: &Launch,
        _restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure> {
        ReadyAuthenticatedGuest::from_observation(
            ReadinessProgress::from_steps([]),
            ExitStatus::Code(1),
        )
    }

    fn execute(
        &mut self,
        _execute: &Execute,
        _guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure> {
        Err(PlatformFailure::new(Recovery::RepairHost))
    }

    fn stop(
        &mut self,
        _stop: &Stop,
        _guest: Option<&mut ReadyAuthenticatedGuest>,
    ) -> Result<PlatformStop, PlatformFailure> {
        Err(PlatformFailure::new(Recovery::RepairHost))
    }

    fn rollback(&mut self, _launch: &Launch) -> Result<(), PlatformFailure> {
        Ok(())
    }
}
