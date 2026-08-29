#![doc = "Durable local state and target-gated host adapters for SOMA."]
#![forbid(unsafe_code)]

mod backend;
mod config;
mod error;
mod file_store;
mod runtime;

#[cfg(test)]
mod test_support;

pub use backend::{BackendProbe, BackendSelection, probe_backend};
pub use config::LocalRuntimeConfig;
pub use error::{LocalFailure, LocalFailureKind};
pub use file_store::FileStateStore;
pub use runtime::LocalRuntime;
