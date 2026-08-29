use std::any::Any;

use soma::{
    BackendFailure, LaunchObservation, LaunchRequest, ResolutionObservation, ResolutionRequest,
};

use super::{MacBackend, MacPreparedWorkload};

impl MacBackend {
    pub(in crate::backend) fn resolve_box(
        &mut self,
        request: ResolutionRequest<'_>,
    ) -> Result<ResolutionObservation<Box<dyn Any + Send>>, BackendFailure> {
        let observation = self.resolve(request)?;
        let (operation_id, source, workload, prepared, elapsed) = observation.into_parts();
        Ok(ResolutionObservation::new(
            operation_id,
            source,
            workload,
            Box::new(prepared),
            elapsed,
        ))
    }

    pub(in crate::backend) fn launch_box(
        &mut self,
        request: &LaunchRequest<'_, Box<dyn Any + Send>>,
    ) -> Result<LaunchObservation, BackendFailure> {
        let prepared = request
            .prepared()
            .downcast_ref::<MacPreparedWorkload>()
            .ok_or_else(|| {
                self.failure(
                    request.operation_id(),
                    soma::BackendFailureKind::WorkloadRejected,
                )
            })?;
        let concrete = LaunchRequest::new(
            request.operation_id(),
            request.instance_id(),
            request.workload(),
            prepared,
            request.shape(),
        );
        self.launch(&concrete)
    }
}
