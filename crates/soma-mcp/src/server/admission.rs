use std::sync::Arc;

use crate::{RuntimeFailure, RuntimeRequest, RuntimeResponse, ToolRuntime};

use super::SomaMcpServer;

impl<R: ToolRuntime> SomaMcpServer<R> {
    pub(super) async fn invoke_runtime(
        &self,
        request: RuntimeRequest,
    ) -> Result<RuntimeResponse, RuntimeFailure> {
        let _permit = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| RuntimeFailure::new(crate::RuntimeFailureKind::Unavailable))?;
        self.runtime.invoke(request).await
    }
}
