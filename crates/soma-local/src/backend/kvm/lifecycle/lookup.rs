//! Finding the Instance a request names, and saying honestly when this process never held it.
//!
//! These are the small decisions a caller cannot make for itself: whether an Instance is absent
//! because it never existed, because another process owns it, or because it has already been
//! released. Keeping them together, and apart from the lifecycle operations that use them, is
//! what stops a failure being reported as the wrong kind.

use soma::{
    BackendFailure, BackendFailureKind, CleanupDisposition, CleanupEvidence, CleanupMethod,
    InstanceId,
};

use super::{KvmBackend, Live};

impl KvmBackend {
    pub(in crate::backend::kvm) fn fail(
        &mut self,
        operation: &soma::OperationId,
        kind: BackendFailureKind,
    ) -> BackendFailure {
        BackendFailure::new(kind, self.clocks.elapsed_ns(operation))
    }

    /// What an operation naming an Instance this process is not driving reports.
    ///
    /// An Instance the Host Runtime still owns is a different fact from an unknown one. It
    /// exists and it is addressable, but its guest session is resident in the process that
    /// launched it, so no session here can serve it. That is a capability this Backend does not
    /// have yet rather than an absent Machine, and it stays missing until the machine runs in
    /// the worker the Host owns instead of inside the launching process.
    pub(in crate::backend::kvm) fn absent_kind(&self, instance: &InstanceId) -> BackendFailureKind {
        if self.ownership.is_live(instance) {
            BackendFailureKind::Unsupported
        } else {
            BackendFailureKind::Unavailable
        }
    }

    /// The live sandbox for `instance`, if this Backend owns one that is still usable.
    ///
    /// A poisoned session is not live: it has already been ended, and reporting it as Ready or
    /// executing against it would attribute work to a machine that is gone.
    pub(in crate::backend::kvm) fn live_for(&mut self, instance: &InstanceId) -> Option<&mut Live> {
        self.live
            .as_mut()
            .filter(|live| &live.instance == instance && live.held.is_usable())
    }

    pub(in crate::backend::kvm) fn take_live(&mut self, instance: &InstanceId) -> Option<Live> {
        if self
            .live
            .as_ref()
            .is_some_and(|live| &live.instance == instance)
        {
            self.live.take()
        } else {
            None
        }
    }
}

/// The dispositions for an Instance this Backend never owned.
///
/// Every resource is `NotOwned` rather than `Complete`: this process holds no record that these
/// resources existed, so it cannot report having released them. A caller can still distinguish
/// this from a real release, which is the point.
pub(in crate::backend::kvm) fn not_owned_evidence() -> CleanupEvidence {
    CleanupEvidence::new(
        CleanupDisposition::NotOwned,
        CleanupDisposition::NotOwned,
        CleanupDisposition::NotOwned,
        CleanupDisposition::NotOwned,
        CleanupDisposition::NotOwned,
    )
    .with_method(CleanupMethod::NotApplicable)
}
