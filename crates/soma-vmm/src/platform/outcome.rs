use crate::{ExitStatus, Recovery};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlatformFailure {
    recovery: Recovery,
}

impl PlatformFailure {
    pub(crate) const fn new(recovery: Recovery) -> Self {
        Self { recovery }
    }

    pub(crate) const fn recovery(self) -> Recovery {
        self.recovery
    }
}

pub(crate) struct PlatformExecution {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl PlatformExecution {
    /// One command that ran to a terminal status.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) const fn completed(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(status: ExitStatus, stdout: Vec<u8>, stderr: Vec<u8>) -> Self {
        Self {
            status,
            stdout,
            stderr,
        }
    }

    pub(crate) fn into_parts(self) -> (ExitStatus, Vec<u8>, Vec<u8>) {
        (self.status, self.stdout, self.stderr)
    }
}

pub(crate) struct PlatformStop {
    guest_acknowledged: bool,
    forced: bool,
}

impl PlatformStop {
    /// One machine that released everything it owned.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) const fn released(guest_acknowledged: bool, forced: bool) -> Self {
        Self {
            guest_acknowledged,
            forced,
        }
    }

    #[cfg(test)]
    pub(crate) const fn for_test(guest_acknowledged: bool, forced: bool) -> Self {
        Self {
            guest_acknowledged,
            forced,
        }
    }

    pub(crate) const fn guest_acknowledged(&self) -> bool {
        self.guest_acknowledged
    }

    pub(crate) const fn forced(&self) -> bool {
        self.forced
    }
}
