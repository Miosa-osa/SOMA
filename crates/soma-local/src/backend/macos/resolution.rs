use soma::{
    BackendFailure, BackendFailureKind, OciDigest, OciPlatform, ResolutionObservation,
    ResolutionRequest, WorkloadIdentity,
};
use soma_macos::ImageReference;

use super::{
    adapter::{MacBackend, MacPreparedWorkload},
    config::control_limits,
};

impl MacBackend {
    pub(in crate::backend) fn resolve(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<MacPreparedWorkload>, BackendFailure> {
        self.clocks.elapsed_ns(request.operation_id());
        let image = ImageReference::new(request.image().as_str()).map_err(|_| {
            self.failure(request.operation_id(), BackendFailureKind::WorkloadRejected)
        })?;
        let resolved = self
            .backend
            .resolve_image(&image, control_limits())
            .map_err(|error| self.map_error(request.operation_id(), &error))?;
        let manifest = OciDigest::parse(resolved.manifest_digest().as_str()).map_err(|_| {
            self.failure(request.operation_id(), BackendFailureKind::WorkloadRejected)
        })?;
        let index = OciDigest::parse(resolved.index_digest().as_str()).map_err(|_| {
            self.failure(request.operation_id(), BackendFailureKind::WorkloadRejected)
        })?;
        let platform = OciPlatform::new(
            resolved.platform().os(),
            resolved.platform().architecture(),
            resolved.platform().variant().map(str::to_owned),
        )
        .map_err(|_| self.failure(request.operation_id(), BackendFailureKind::WorkloadRejected))?;
        let identity = WorkloadIdentity::new(manifest, platform, None).with_index_digest(index);
        let prepared = MacPreparedWorkload {
            image,
            identity: identity.clone(),
        };
        Ok(ResolutionObservation::new(
            request.operation_id().clone(),
            request.source_fingerprint().clone(),
            identity,
            prepared,
            self.clocks.elapsed_ns(request.operation_id()),
        ))
    }
}
