//! Suspending and resuming managed Machines (ADR 0034), and reading the host's capacity.
//!
//! The store lives on the KVM host only; elsewhere a suspension is refused as unsupported and the
//! Instance keeps running.

use soma::MachineLaunch;

use super::LocalRuntime;
use crate::{ResumeRequest, SuspendRequest, Suspended, SuspensionFailure};

impl LocalRuntime {
    /// Reads one managed Machine into a sealed Suspension and ends it (ADR 0034).
    ///
    /// # Errors
    ///
    /// Returns [`SuspensionFailure::Managed`] with `Backend(Unsupported)` when the Machine cannot
    /// be suspended, in which case it keeps running; any other failure after the read began has
    /// ended it.
    pub fn suspend_machine(
        &mut self,
        request: &SuspendRequest,
    ) -> Result<Suspended, SuspensionFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return self.suspend_here(request);
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = request;
            Err(crate::suspension::unsupported())
        }
    }

    /// Resumes a Suspension as a new managed Machine, then destroys the Suspension.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; the Suspension is kept whenever no Machine resumed from it.
    pub fn resume_suspension(
        &mut self,
        request: &ResumeRequest,
    ) -> Result<MachineLaunch, SuspensionFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return self.resume_here(request);
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = request;
            Err(crate::suspension::unsupported())
        }
    }

    /// Destroys a Suspension without resuming it.
    ///
    /// # Errors
    ///
    /// Returns the typed failure when the identity is malformed or the store refuses.
    pub fn destroy_suspension(&self, suspension_id: &str) -> Result<(), SuspensionFailure> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        return self.destroy_here(suspension_id);
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = suspension_id;
            Err(crate::suspension::unsupported())
        }
    }

    /// What this host has left for sandboxes, read now.
    #[must_use]
    pub fn capacity(&self) -> crate::capacity::HostCapacity {
        crate::capacity::read(&self.state_root)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl LocalRuntime {
    /// Reads one managed Machine into a sealed Suspension and ends it (ADR 0034).
    ///
    /// # Errors
    ///
    /// Returns [`SuspensionFailure::Managed`] with `Backend(Unsupported)` when the
    /// Machine cannot be suspended, in which case it keeps running; any other failure after the
    /// read began has ended it.
    fn suspend_here(&mut self, request: &SuspendRequest) -> Result<Suspended, SuspensionFailure> {
        let started = std::time::Instant::now();
        let (id, staging) = crate::suspension::Suspensions::at(&self.state_root)
            .stage()
            .map_err(|error| SuspensionFailure::Store(error.to_string()))?;
        let suspended = match self
            .engine
            .suspend_machine(&crate::suspension::engine_request(request, &staging))
        {
            Ok(suspended) => suspended,
            Err(failure) => {
                let _ignored = std::fs::remove_dir_all(&staging);
                return Err(failure.into());
            }
        };
        // A read that started and failed ends the Instance with a Failed receipt: the machine is
        // gone and there is nothing to seal.
        if suspended.receipt().terminal_status() != &soma::TerminalStatus::Suspended {
            let _ignored = std::fs::remove_dir_all(&staging);
            return Err(SuspensionFailure::Lost(Box::new(
                suspended.receipt().clone(),
            )));
        }
        let read = started.elapsed();
        let sealed = crate::suspension::Suspensions::at(&self.state_root).seal(
            id,
            &staging,
            request,
            suspended.receipt(),
        );
        eprintln!(
            "soma: suspension of {} read in {} ms, sealed in {} ms",
            request.instance_id.as_str(),
            read.as_millis(),
            started.elapsed().saturating_sub(read).as_millis()
        );
        sealed
    }

    /// Resumes a Suspension as a new managed Machine, then destroys the Suspension.
    ///
    /// # Errors
    ///
    /// Returns the typed failure; the Suspension is kept whenever no Machine resumed from it.
    fn resume_here(&mut self, request: &ResumeRequest) -> Result<MachineLaunch, SuspensionFailure> {
        let suspensions = crate::suspension::Suspensions::at(&self.state_root);
        let opened = suspensions.open(request)?;
        let launched = self.engine.launch_machine(opened.launch(request));
        // The launch removes the members once it holds them; a refusal before that leaves them.
        let _ignored = std::fs::remove_dir_all(&opened.directory);
        let launch = launched?;
        // One Suspension resumes once: a second Instance from the same members would share a
        // private head's history and every secret the first one held.
        let _ignored = suspensions.destroy(&opened.id.hex());
        Ok(launch)
    }

    /// Destroys a Suspension without resuming it.
    ///
    /// # Errors
    ///
    /// Returns the typed failure when the identity is malformed or the store refuses.
    fn destroy_here(&self, suspension_id: &str) -> Result<(), SuspensionFailure> {
        crate::suspension::Suspensions::at(&self.state_root).destroy(suspension_id)
    }
}
