mod execute;
mod fault;
mod launch;
mod stop;

use crate::{
    InstanceId,
    operation::OperationLedger,
    platform::{Platform, ReadyAuthenticatedGuest, UnavailablePlatform},
};

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

    #[cfg(test)]
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
