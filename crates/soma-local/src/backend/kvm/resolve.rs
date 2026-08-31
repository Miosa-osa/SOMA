//! Resolving a request to a Generation this host prepared before the request arrived.
//!
//! Resolution reads. It does not acquire an OCI image, and it does not compile a Generation,
//! because both belong before demand rather than on the request path. A host that prepared
//! nothing for the requested image refuses the request instead of building one for it.

use soma::{
    BackendFailure, BackendFailureKind, OciDigest, OperationId, ResolutionObservation,
    ResolutionRequest, WorkloadIdentity,
};

use super::{
    KvmBackend,
    prepared::{self, PreparedError, PreparedGeneration},
};

/// The failure a caller sees for each way resolution can end without a Generation.
///
/// A host that prepares nothing and a host whose prepared entry is damaged are different
/// operator problems, so they do not collapse into one kind.
#[allow(dead_code, reason = "used by resolve, which is not yet wired")]
const fn kind_for(error: &PreparedError) -> BackendFailureKind {
    match error {
        // The host is prepared, just not for this image. That is the only condition the
        // request itself is responsible for.
        PreparedError::NotPrepared => BackendFailureKind::WorkloadRejected,
        // Everything else is a property of the host: it prepares nothing, its root cannot be
        // read, or what claims this reference is damaged, duplicated, or a link. Each is an
        // operator fault rather than a fault in the request.
        // A Candidate is refused because no certification gate exists to verify it, which is
        // a missing capability rather than a fault in the host's files.
        PreparedError::Uncertified => BackendFailureKind::Unsupported,
        PreparedError::StoreUnset
        | PreparedError::StoreUnreadable
        | PreparedError::Damaged
        | PreparedError::Ambiguous
        | PreparedError::Linked => BackendFailureKind::Unavailable,
    }
}

impl KvmBackend {
    /// Not yet reachable from dispatch.
    ///
    /// Wiring this alone would change what the command line reports for an explicit KVM run
    /// from unsupported to unavailable, which would send an operator to prepare a Generation
    /// store that `launch` still could not use. Dispatch adopts all five operations together.
    #[allow(dead_code, reason = "wired when the lifecycle stops failing closed")]
    pub(in crate::backend) fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Box<dyn std::any::Any + Send>>, BackendFailure> {
        let operation = request.operation_id();
        let reference = request.image().as_str();
        let found = prepared::find(prepared::store_root().as_deref(), reference)
            .map_err(|error| self.failure(operation, kind_for(&error)))?;
        let workload = identity(&found)
            .ok_or_else(|| self.failure(operation, BackendFailureKind::WorkloadRejected))?;
        let elapsed = self.clocks.elapsed_ns(operation);
        Ok(ResolutionObservation::new(
            operation.clone(),
            request.source_fingerprint().clone(),
            workload,
            Box::new(found) as Box<dyn std::any::Any + Send>,
            elapsed,
        ))
    }

    #[allow(dead_code, reason = "used by resolve, which is not yet wired")]
    fn failure(&mut self, operation: &OperationId, kind: BackendFailureKind) -> BackendFailure {
        BackendFailure::new(kind, self.clocks.elapsed_ns(operation))
    }
}

/// The workload identity a prepared Generation reports.
///
#[allow(dead_code, reason = "used by resolve, which is not yet wired")]
fn identity(found: &PreparedGeneration) -> Option<WorkloadIdentity> {
    let source = &found.manifest.source;
    let digest = OciDigest::parse(source.oci_manifest_digest.to_string()).ok()?;
    Some(WorkloadIdentity::new(
        digest,
        source.platform.clone(),
        Some(found.id.clone()),
    ))
}
