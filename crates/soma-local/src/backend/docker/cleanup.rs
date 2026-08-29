use soma::{CleanupEvidence, CleanupMethod, CleanupObservation, CleanupRequest, CleanupTimes};

use super::DockerBackend;
use super::container::{container_name, remove};

impl DockerBackend {
    pub(in crate::backend) fn cleanup(
        &mut self,
        request: CleanupRequest<'_>,
    ) -> CleanupObservation {
        let key = request.instance_id().as_str().to_owned();
        let started = self.clocks.elapsed_ns(request.operation_id());
        let complete = if self.already_cleaned.remove(&key) {
            true
        } else {
            remove(&container_name(&key))
        };
        let evidence = if complete {
            CleanupEvidence::complete_owned_machine().with_method(CleanupMethod::Forced)
        } else {
            CleanupEvidence::incomplete_owned_machine().with_method(CleanupMethod::Forced)
        };
        CleanupObservation::new(
            request.operation_id().clone(),
            request.instance_id().clone(),
            evidence,
            CleanupTimes::new(started, self.clocks.elapsed_ns(request.operation_id())),
        )
    }
}
