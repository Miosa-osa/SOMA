use std::{
    ops::{Deref, DerefMut},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

use soma::{
    DestroyMachineRequest, ExecuteMachineRequest, FileMachineRequest, InspectMachineRequest,
    LaunchMachineRequest, ManagedFailure, PtyMachineRequest, SandboxEntry, StopMachineRequest,
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

    fn file(&mut self, request: FileMachineRequest) -> Result<FileOutcome, ManagedFailure> {
        self.deref_mut().file(request)
    }

    fn terminal(&mut self, request: PtyMachineRequest) -> Result<TerminalOutcome, ManagedFailure> {
        self.deref_mut().terminal(request)
    }

    fn list(&mut self) -> Result<Vec<SandboxEntry>, ManagedFailure> {
        self.deref_mut().list()
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
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };

    use super::FacadePool;

    #[test]
    fn every_facade_is_opened_before_the_pool_is_published() {
        let mut opened = 0;
        let pool = FacadePool::open(4, || {
            opened += 1;
            Ok::<_, ()>(opened)
        })
        .expect("open the pool");

        assert_eq!(opened, 4);
        assert!(
            *pool
                .acquire_timeout(Duration::from_millis(10))
                .expect("one facade is available")
                > 0
        );
    }

    #[test]
    fn concurrent_callers_receive_distinct_facades() {
        let pool = FacadePool::open(8, || Ok::<_, ()>(())).expect("open the pool");
        let gate = Arc::new(Barrier::new(8));
        let mut callers = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            let gate = Arc::clone(&gate);
            callers.push(thread::spawn(move || {
                let _lease = pool
                    .acquire_timeout(Duration::from_millis(10))
                    .expect("one facade per caller");
                gate.wait();
            }));
        }
        for caller in callers {
            caller.join().expect("caller completed");
        }
    }

    #[test]
    fn bounded_acquisition_times_out_and_a_returned_lease_is_reused() {
        let pool = FacadePool::open(1, || Ok::<_, ()>(7)).expect("open the pool");
        let held = pool
            .acquire_timeout(Duration::from_millis(10))
            .expect("initial facade");
        assert!(pool.acquire_timeout(Duration::ZERO).is_none());
        drop(held);
        assert_eq!(
            *pool
                .acquire_timeout(Duration::from_millis(10))
                .expect("returned facade is available"),
            7
        );
    }
}
