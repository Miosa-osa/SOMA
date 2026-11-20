//! The windows this runner closes on its own: the ones that ran out, and after a restart all of
//! them, because the timers that would have closed them died with the previous process.

use std::time::Instant;

use soma_local::EgressSetFailure;

use crate::runner::ids::SandboxId;

use super::{CLOSED, Flip, Runner};

impl Runner {
    /// Closes every window that has run out. A close the host does not confirm is tried again on
    /// the next sweep; a sandbox that is gone has nothing left to close.
    pub(in crate::runner::service) async fn close_expired_windows(&self) {
        for id in self.windows.expired(Instant::now()) {
            let Some(instance) = id.instance_id() else {
                self.windows.forget(&id);
                continue;
            };
            match self.flip(&instance, CLOSED).await {
                Flip::Done(_) => self.windows.closed(&id),
                Flip::Failed(EgressSetFailure::NotFound) => self.windows.forget(&id),
                Flip::Failed(failure) => {
                    eprintln!(
                        "soma-api: runner could not close the egress window of {id}: {failure:?}"
                    );
                }
                Flip::Busy => {}
            }
        }
    }

    /// After a restart the runner does not know which windows were open, and the timers died with
    /// it. Asking every recovered sandbox to deny closes whatever was open; a sandbox that holds
    /// no window answers that there is nothing to change, which is ignored. The ones that
    /// answered are windowed, and are recorded as such.
    pub(in crate::runner::service) async fn close_recovered_windows(
        &self,
        recovered: &[SandboxId],
    ) {
        for id in recovered {
            let Some(instance) = id.instance_id() else {
                continue;
            };
            if let Flip::Done(_) = self.flip(&instance, CLOSED).await {
                self.windows.created(id, "denied", true);
            }
        }
    }
}
