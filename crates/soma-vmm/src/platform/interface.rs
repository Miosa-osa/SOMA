use crate::{Execute, Launch, Stop};

use super::{
    PlatformExecution, PlatformFailure, PlatformStop, ReadinessFailure, ReadyAuthenticatedGuest,
    RestoreFailure, RestoredMachine,
};

pub(crate) trait Platform: Send {
    /// Verifies the exact Generation and restores one private machine as one cohesive operation.
    fn verify_and_restore(&mut self, launch: &Launch) -> Result<RestoredMachine, RestoreFailure>;

    /// Authenticates, repairs, and probes the restored guest through one fused adapter operation.
    fn authenticate_repair_and_ready(
        &mut self,
        launch: &Launch,
        restored: RestoredMachine,
    ) -> Result<ReadyAuthenticatedGuest, ReadinessFailure>;

    fn execute(
        &mut self,
        execute: &Execute,
        guest: &mut ReadyAuthenticatedGuest,
    ) -> Result<PlatformExecution, PlatformFailure>;

    fn stop(
        &mut self,
        stop: &Stop,
        guest: Option<&mut ReadyAuthenticatedGuest>,
    ) -> Result<PlatformStop, PlatformFailure>;

    fn rollback(&mut self, launch: &Launch) -> Result<(), PlatformFailure>;
}
