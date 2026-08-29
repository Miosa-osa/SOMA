//! In-process launcher and resource broker for deterministic tests without a kernel.
//!
//! The launcher keeps a shared process table so a second pool can "restart" over the same
//! ledger and probe the processes the first one left behind; the broker leases heads
//! through the real `soma-storage` ledger and derives launch identities the way the
//! network broker would, over socket pairs instead of TAP devices.

mod broker;
mod launcher;
mod table;

pub use broker::{BrokerCounters, InProcessBroker, SterileResources};
pub use launcher::{FaultPlan, InProcessHandle, InProcessLauncher, InjectedFault};
pub use table::{Process, ProcessTable};
