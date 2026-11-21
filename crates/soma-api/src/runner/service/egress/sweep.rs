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

    /// Forgets the network record of every sandbox this runner no longer holds at all.
    ///
    /// The record is made at create, before the launch. A destroy and a failed waiting create
    /// forget it on their way out, but a launch that fails after an early answer leaves a
    /// tombstone that the reaper's sweep drops later, and a create abandoned before its launch
    /// drops its record outright; neither passes through anything that forgets. Run after the
    /// sweep, this keeps the window table no larger than the sandbox table. The two tables are
    /// read one after the other, never one inside the other's lock.
    pub(in crate::runner::service) fn forget_gone_windows(&self) {
        for id in self.windows.known() {
            if !self.sandboxes.holds(&id) {
                self.windows.forget(&id);
            }
        }
    }
}
