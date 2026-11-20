mod adapter;
mod boxed;
mod config;
mod evidence;
mod execution;
mod failure;
mod lifecycle;
mod network;
mod resolution;

pub(crate) use adapter::{MacBackend, MacPreparedWorkload};
pub(crate) use network::serves as serves_network;
