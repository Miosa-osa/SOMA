mod execute;
mod fault;
mod launch;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod pty;
mod stop;

use crate::{
    InstanceId,
    operation::OperationLedger,
    platform::{Platform, ReadyAuthenticatedGuest, UnavailablePlatform},
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::platform::KvmPlatform;

pub struct Machine {
    platform: Box<dyn Platform>,
    state: State,
    operations: OperationLedger,
    instance_id: Option<InstanceId>,
    guest: Option<ReadyAuthenticatedGuest>,
}

impl Machine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            platform: Box::new(UnavailablePlatform),
            state: State::New,
            operations: OperationLedger::default(),
            instance_id: None,
            guest: None,
        }
    }

    /// One Machine backed by the KVM provider, built from the sealed descriptor table.
    ///
    /// Returns `None` when the manifest does not name a whole machine, which is a launcher
    /// fault rather than a request fault: a worker that served the contract with a partial
    /// table would report a Machine that cannot exist.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[must_use]
    pub fn on_jailed_kvm(manifest: &soma_jail::DescriptorManifest) -> Option<Self> {
        Some(Self::with_platform(KvmPlatform::adopt(manifest)?))
    }

    #[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
    pub(crate) fn with_platform(platform: impl Platform + 'static) -> Self {
        Self {
            platform: Box::new(platform),
            state: State::New,
            operations: OperationLedger::default(),
            instance_id: None,
            guest: None,
        }
    }
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    New,
    Launching,
    Ready,
    Failed,
    Stopped,
}
