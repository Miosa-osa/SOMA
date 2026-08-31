//! The persistent Instance ownership accepted by ADR 0031.
//!
//! The pool owns prepared workers; it does not own Instances, so an Instance created by one
//! command died with the process that created it and could never be addressed again.
//! The [`Runtime`] closes that gap: it is the single private owner of every live Instance of
//! one Host, it keys them by their exact [`InstanceId`], and it outlives every client because
//! the daemon, not the caller, holds it.
//!
//! Durability and idempotency are the pool's, not a second scheme: one Launch is one claim,
//! so one operation identity yields one Instance, a replay of that identity is answered from
//! the same record, and a changed intent conflicts before any resource moves.
//! The one fact the Runtime adds is the Instance-to-worker binding, and the durable ledger
//! already records it, which is why a terminal Instance is still provable long after the
//! binding itself is gone.

mod error;
mod launch;
mod table;
mod terminal;

use std::sync::{Arc, Mutex, MutexGuard};

pub use error::InstanceError;
pub use table::{InstanceView, Page};
pub use terminal::TerminalReceipt;

use table::LiveTable;

use crate::{InstanceId, LeaseGeneration, Pool, ResourceBroker, WorkerId, WorkerLauncher};

/// The largest number of Instances one [`Runtime::list`] page reports.
///
/// The page is bounded for the same reason every other frame is: one reply is one frame, and
/// a Host admitted for many Instances must not be able to make the daemon build a frame no
/// client can receive.
pub const MAX_LISTED: usize = 16;

/// What one accepted Launch produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Launched {
    /// The Instance is live and addressable.
    Live(InstanceView),
    /// The operation already holds a worker whose launch page this process never delivered,
    /// so the Runtime cannot repeat it; the client destroys the Instance and launches again
    /// under a fresh operation.
    Replayed {
        /// The worker.
        worker: WorkerId,
        /// The lease generation.
        lease_generation: LeaseGeneration,
    },
}

/// The private owner of every live Instance of one Host.
pub struct Runtime<L: WorkerLauncher, R: ResourceBroker> {
    pool: Arc<Pool<L, R>>,
    live: Mutex<LiveTable>,
}

impl<L: WorkerLauncher, R: ResourceBroker> Runtime<L, R> {
    /// Takes ownership of `pool` and starts with no live Instance.
    #[must_use]
    pub fn new(pool: Arc<Pool<L, R>>) -> Self {
        Self {
            pool,
            live: Mutex::new(LiveTable::default()),
        }
    }

    /// Returns the pool the Runtime allocates from.
    #[must_use]
    pub const fn pool(&self) -> &Arc<Pool<L, R>> {
        &self.pool
    }

    /// Returns the live Instance with this identity.
    #[must_use]
    pub fn get(&self, instance: InstanceId) -> Option<InstanceView> {
        self.reclaim();
        self.locked().get(instance)
    }

    /// Reports at most [`MAX_LISTED`] live Instances after `after` in identity order.
    #[must_use]
    pub fn list(&self, after: Option<InstanceId>) -> Page {
        self.reclaim();
        self.locked().page(after, MAX_LISTED)
    }

    /// Returns how many Instances the Runtime currently owns.
    #[must_use]
    pub fn live(&self) -> usize {
        self.reclaim();
        self.locked().len()
    }

    /// Drops every binding whose worker the pool proves is gone.
    ///
    /// Ownership is released only against evidence: a worker the pool no longer knows, or one
    /// it reports in a terminal phase, cannot still be serving an Instance, and keeping the
    /// binding would let a lookup name a Machine that no longer exists.
    fn reclaim(&self) {
        let mut table = self.locked();
        table.retain(|worker| {
            self.pool
                .inspect(worker)
                .is_some_and(|view| !view.phase.is_terminal())
        });
    }

    pub(crate) fn bind(&self, view: InstanceView) -> InstanceView {
        self.locked().bind(view);
        view
    }

    pub(crate) fn locked(&self) -> MutexGuard<'_, LiveTable> {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
