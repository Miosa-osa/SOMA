use crate::{OciImage, OperationId, RequestFingerprint, WorkloadIdentity};

#[derive(Clone, Copy)]
pub struct ResolutionRequest<'a> {
    operation_id: &'a OperationId,
    image: &'a OciImage,
    source_fingerprint: &'a RequestFingerprint,
}

impl<'a> ResolutionRequest<'a> {
    pub(crate) const fn new(
        operation_id: &'a OperationId,
        image: &'a OciImage,
        source_fingerprint: &'a RequestFingerprint,
    ) -> Self {
        Self {
            operation_id,
            image,
            source_fingerprint,
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> &OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn image(&self) -> &OciImage {
        self.image
    }

    #[must_use]
    pub const fn source_fingerprint(&self) -> &RequestFingerprint {
        self.source_fingerprint
    }
}

pub struct ResolutionObservation<P> {
    operation_id: OperationId,
    source_fingerprint: RequestFingerprint,
    workload: WorkloadIdentity,
    prepared: P,
    resolved_at_ns: u64,
}

impl<P> ResolutionObservation<P> {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        source_fingerprint: RequestFingerprint,
        workload: WorkloadIdentity,
        prepared: P,
        resolved_at_ns: u64,
    ) -> Self {
        Self {
            operation_id,
            source_fingerprint,
            workload,
            prepared,
            resolved_at_ns,
        }
    }

    pub fn into_parts(self) -> (OperationId, RequestFingerprint, WorkloadIdentity, P, u64) {
        (
            self.operation_id,
            self.source_fingerprint,
            self.workload,
            self.prepared,
            self.resolved_at_ns,
        )
    }
}
