//! SOMA snapshot format v1: manifest and state codec, memory-object descriptor, and
//! compatibility contract.
//!
//! This module tree is pure encoding, validation, and ordering policy.
//! It performs no KVM ioctl and never captures or restores a live machine.
//! Live capture and restore belong to a later slice that consumes these contracts.

pub mod capture;
pub mod compatibility;
pub mod device_state;
mod digest;
pub mod kvm_state;
pub mod manifest;
pub mod memory;
pub mod restore;
pub mod section;
mod wire;

pub use digest::Digest;
pub use wire::WireError;
