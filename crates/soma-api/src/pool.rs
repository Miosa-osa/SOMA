use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, FileMachineRequest, InspectMachineRequest,
    LaunchMachineRequest, MachineListing, ManagedFailure, PtyMachineRequest, StopMachineRequest,
};

use crate::facade::{
    CommandOutcome, FileOutcome, LifecycleOutcome, SandboxFacade, SandboxSnapshot, TerminalOutcome,
};

/// A bounded set of expensive facades opened before the service accepts traffic.
pub struct FacadePool<F> {
    inner: Arc<Inner<F>>,
}

struct Inner<F> {
    available: Mutex<Vec<F>>,
    returned: Condvar,
}

impl<F> FacadePool<F> {
    /// Opens exactly `capacity` facades before returning a usable pool.
    ///
    /// # Panics
    ///
    /// Panics when `capacity` is zero.
    ///
    /// # Errors
    ///
    /// Returns the first opener failure.
    pub fn open<E>(capacity: usize, mut opener: impl FnMut() -> Result<F, E>) -> Result<Self, E> {
        assert!(capacity > 0, "a facade pool must have capacity");
        let mut available = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            available.push(opener()?);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                available: Mutex::new(available),
                returned: Condvar::new(),
            }),
        })
    }

    /// Lends one facade before `timeout` expires, or refuses bounded overload.
    #[must_use]
    pub fn acquire_timeout(&self, timeout: Duration) -> Option<FacadeLease<F>> {
        let deadline = Instant::now().checked_add(timeout)?;
        let mut available = self
            .inner
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(facade) = available.pop() {
                return Some(FacadeLease {
                    facade: Some(facade),
                    pool: Arc::clone(&self.inner),
                });
            }
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let (guard, result) = self
                .inner
                .returned
                .wait_timeout(available, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            available = guard;
            if result.timed_out() && available.is_empty() {
                return None;
            }
        }
    }
}

impl<F> Clone for FacadePool<F> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// Exclusive use of one preopened facade for one request.
pub struct FacadeLease<F> {
    facade: Option<F>,
    pool: Arc<Inner<F>>,
}

impl<F> Deref for FacadeLease<F> {
    type Target = F;

    fn deref(&self) -> &Self::Target {
        self.facade
            .as_ref()
            .expect("a live facade lease always owns its facade")
    }
}

impl<F> DerefMut for FacadeLease<F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.facade
            .as_mut()
            .expect("a live facade lease always owns its facade")
    }
}

impl<F> Drop for FacadeLease<F> {
    fn drop(&mut self) {
        let Some(facade) = self.facade.take() else {
            return;
        };
        let mut available = self
            .pool
            .available
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        available.push(facade);
        self.pool.returned.notify_one();
    }
}

impl<F: SandboxFacade> SandboxFacade for FacadeLease<F> {
    fn hosts_addressable_sandboxes(&self) -> bool {
        self.deref().hosts_addressable_sandboxes()
    }

    fn launch(
        &mut self,
        request: LaunchMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.deref_mut().launch(request)
    }

    fn inspect(
        &mut self,
        request: InspectMachineRequest,
    ) -> Result<SandboxSnapshot, ManagedFailure> {
        self.deref_mut().inspect(request)
    }

    fn execute(
        &mut self,
        request: ExecuteMachineRequest,
    ) -> Result<CommandOutcome, ManagedFailure> {
        self.deref_mut().execute(request)
    }

    fn execute_streaming(
        &mut self,
        request: ExecuteMachineRequest,
        sink: soma::OutputSink<'_>,
    ) -> Result<CommandOutcome, ManagedFailure> {
        self.deref_mut().execute_streaming(request, sink)
    }

    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        self.deref_mut().file(request)
    }

    fn terminal(&mut self, request: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        self.deref_mut().terminal(request)
    }

    fn list(&mut self) -> Result<MachineListing, ManagedFailure> {
        self.deref_mut().list()
    }

    fn terminal_census(&mut self) -> Result<soma::TerminalCensus, ManagedFailure> {
        self.deref_mut().terminal_census()
    }

    fn release(
        &mut self,
        instance_id: &soma::InstanceId,
    ) -> Result<soma::MachineRelease, ManagedFailure> {
        self.deref_mut().release(instance_id)
    }

    fn stop(&mut self, request: StopMachineRequest) -> Result<LifecycleOutcome, ManagedFailure> {
        self.deref_mut().stop(request)
    }

    fn destroy(
        &mut self,
        request: DestroyMachineRequest,
    ) -> Result<LifecycleOutcome, ManagedFailure> {
        self.deref_mut().destroy(request)
    }

    fn resize(
        &mut self,
        instance_id: &soma::InstanceId,
        request: soma_local::ResizeRequest,
    ) -> Result<soma_local::ResizeOutcome, soma_local::ResizeFailure> {
        self.deref_mut().resize(instance_id, request)
    }

    fn set_egress(
        &mut self,
        instance_id: &soma::InstanceId,
        request: soma_local::EgressSetRequest,
    ) -> Result<soma_local::EgressSetOutcome, soma_local::EgressSetFailure> {
        self.deref_mut().set_egress(instance_id, request)
    }

    fn tunnel(
        &mut self,
        instance_id: &soma::InstanceId,
        port: u16,
    ) -> Result<std::os::unix::net::UnixStream, soma_local::TunnelFailure> {
        self.deref_mut().tunnel(instance_id, port)
    }

    fn volume(
        &mut self,
        operation: crate::volumes::VolumeOperation,
        body: &crate::volumes::VolumeBody,
    ) -> Result<crate::volumes::VolumeReport, crate::volumes::VolumeFailure> {
        self.deref_mut().volume(operation, body)
    }

    fn volume_owned(
        &mut self,
        instance: &str,
    ) -> Result<Option<crate::volumes::OwnedVolume>, crate::volumes::VolumeFailure> {
        self.deref_mut().volume_owned(instance)
    }

    fn volume_binding(
        &mut self,
        instance: &str,
    ) -> Result<crate::volumes::VolumeBinding, crate::volumes::VolumeFailure> {
        self.deref_mut().volume_binding(instance)
    }
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;
