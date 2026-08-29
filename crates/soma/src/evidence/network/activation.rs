use serde::{Deserialize, Serialize};

/// How host ingress ownership transitioned from reservation to the active runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortActivationClass {
    /// No host ingress endpoints were requested.
    NotApplicable,
    /// The network broker handed already-bound sockets or descriptors to the runtime atomically.
    AtomicSocketHandoff,
    /// The adapter verified a runtime rebind after releasing its reservation.
    VerifiedRuntimeRebind,
}
