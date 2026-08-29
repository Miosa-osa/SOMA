//! The five v1 device models behind the common virtio-mmio transport.
//!
//! Each model owns one parser over validated descriptor chains, one backend
//! seam that accepts only validated operations, and one fixed snapshot record.
//! Nothing here touches KVM, eventfds, or a real guest.

pub mod block;
pub mod segments;
pub mod service;

#[cfg(test)]
pub(crate) mod harness;
