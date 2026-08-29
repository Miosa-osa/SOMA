use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementClass {
    FacadeRunEndToEnd,
    FacadeManagedLaunch,
    FacadeManagedCommand,
    FacadeManagedStop,
    FacadeManagedInspect,
    FacadeManagedDestroy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeasurementBoundary {
    class: MeasurementClass,
    clock: MeasurementClock,
    origin: MeasurementOrigin,
    terminal: MeasurementTerminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementClock {
    BackendMonotonicElapsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementOrigin {
    FacadeRequestAccepted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementTerminal {
    ReceiptTerminal,
}

impl MeasurementBoundary {
    pub(crate) fn for_class(class: MeasurementClass) -> Self {
        Self {
            class,
            clock: MeasurementClock::BackendMonotonicElapsed,
            origin: MeasurementOrigin::FacadeRequestAccepted,
            terminal: MeasurementTerminal::ReceiptTerminal,
        }
    }

    #[must_use]
    pub const fn class(&self) -> MeasurementClass {
        self.class
    }

    #[must_use]
    pub const fn clock(&self) -> MeasurementClock {
        self.clock
    }

    #[must_use]
    pub const fn origin(&self) -> MeasurementOrigin {
        self.origin
    }

    #[must_use]
    pub const fn terminal(&self) -> MeasurementTerminal {
        self.terminal
    }
}
