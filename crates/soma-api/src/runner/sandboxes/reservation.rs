//! A create request's claim on the sandbox it reserved.

use std::sync::Arc;

use super::{SandboxId, Sandboxes, records::Phase};

/// The sandbox one create reserved, held until the request that reserved it is done with it.
///
/// The tenant slot is taken before the launch and belongs to the sandbox once there is one, so
/// between those two moments it belongs to this request. A request that ends without settling -
/// a client that resets the stream mid-launch, a handler that is dropped, a panic - would
/// otherwise leave the slot held by a record nothing will ever look at again, and the tenant's
/// share on this runner would shrink by one for the life of the process.
pub struct Reservation {
    sandboxes: Arc<Sandboxes>,
    id: SandboxId,
    /// Cleared the moment the record belongs to a launch rather than to this request.
    armed: bool,
}

impl Reservation {
    #[must_use]
    pub(in crate::runner) fn new(sandboxes: Arc<Sandboxes>, id: SandboxId) -> Self {
        Self {
            sandboxes,
            id,
            armed: true,
        }
    }

    /// The record belongs to the sandbox now - a launch holds it - so this request no longer
    /// owns it and must not give its slot back.
    pub(in crate::runner) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            self.sandboxes.abandon_reservation(&self.id);
        }
    }
}

impl Sandboxes {
    /// Forgets a reservation the creating request still owns, giving its tenant slot back.
    ///
    /// A record that has moved on - handed to a launch, confirmed, failed, destroyed - belongs to
    /// whoever moved it, so this leaves it alone. It is called however a create request ends,
    /// including when the request is dropped rather than answered.
    pub(in crate::runner) fn abandon_reservation(&self, id: &SandboxId) {
        let mut records = self.lock();
        let reserved = records
            .get(id)
            .is_some_and(|record| matches!(record.phase, Phase::Creating));
        if !reserved {
            return;
        }
        if let Some(record) = records.remove(id) {
            // An admitted create that never settled leaves no other trace: the journal is written
            // with the response, and this request never got one. Say what it reached and how long
            // it had run, so the next incident is traceable rather than invisible.
            eprintln!(
                "soma-api: runner create {id} was abandoned before it settled; tenant={} key={} phase=creating elapsed_ms={}",
                record.owner.tenant_id,
                record.owner.key_id.as_deref().unwrap_or("-"),
                record.owner.created.elapsed().as_millis(),
            );
            record.owner.give_back_slot();
        }
    }
}
