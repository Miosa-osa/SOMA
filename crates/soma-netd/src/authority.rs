//! Who may reach the privileged control socket and which operations each peer may request.
//!
//! The identity is kernel-derived: the broker reads it from the connected socket rather than
//! from any request field, so a peer cannot name itself.
//! Admission is a two-step decision.
//! A peer that is not admitted at all never reaches a decoder, and an admitted peer still needs
//! the [`Capability`] its exact operation requires.
//!
//! The production handoff is one lifecycle peer, `soma-hostd`, which claims, activates, and
//! releases the network bundle of every Machine it owns, plus operator tooling that may only
//! reconcile.
//! The jailed VMM never speaks this protocol: it receives one already-open TAP descriptor from
//! `soma-hostd` and holds no control capability at all.

use std::collections::BTreeSet;

use crate::{Error, Request};

/// The kernel-derived identity of one connected control peer.
///
/// The process identifier is retained as evidence only; ownership decisions use the user
/// identity, which the kernel stamps at connect time and a peer cannot forge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerIdentity {
    uid: u32,
    gid: u32,
    pid: i32,
}

impl PeerIdentity {
    /// Records one peer credential exactly as the kernel reported it.
    #[must_use]
    pub const fn new(uid: u32, gid: u32, pid: i32) -> Self {
        Self { uid, gid, pid }
    }

    /// Returns the peer's user identity.
    #[must_use]
    pub const fn uid(&self) -> u32 {
        self.uid
    }

    /// Returns the peer's primary group identity.
    #[must_use]
    pub const fn gid(&self) -> u32 {
        self.gid
    }

    /// Returns the peer's process identity, which is evidence rather than authority.
    #[must_use]
    pub const fn pid(&self) -> i32 {
        self.pid
    }
}

/// The application capability one control operation requires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    /// Claim, activate, and release the network bundle of one owned Machine.
    Lifecycle,
    /// Compare the durable ownership ledger with kernel state.
    Reconcile,
}

impl Capability {
    /// Returns the capability this exact request requires.
    #[must_use]
    pub const fn required_for(request: &Request) -> Self {
        match request {
            Request::Claim { .. } | Request::Activate { .. } | Request::Release { .. } => {
                Self::Lifecycle
            }
            Request::Reconcile => Self::Reconcile,
        }
    }
}

/// Which local peers may reach the control socket and what each of them may request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlAuthority {
    owner: u32,
    group: u32,
    lifecycle: BTreeSet<u32>,
    reconcile: BTreeSet<u32>,
}

impl ControlAuthority {
    /// Builds the authority the listener enforces.
    ///
    /// `owner` and `group` must own the socket directory and the socket node; `lifecycle` and
    /// `reconcile` name the user identities admitted for each capability.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidProfile`] when no user identity is admitted at all, because an
    /// authority that admits nobody cannot serve the Machines it was started for.
    pub fn new(
        owner: u32,
        group: u32,
        lifecycle: &[u32],
        reconcile: &[u32],
    ) -> Result<Self, Error> {
        if lifecycle.is_empty() && reconcile.is_empty() {
            return Err(Error::InvalidProfile("control authority admits nobody"));
        }
        Ok(Self {
            owner,
            group,
            lifecycle: lifecycle.iter().copied().collect(),
            reconcile: reconcile.iter().copied().collect(),
        })
    }

    /// Returns the user identity that must own the socket directory and node.
    #[must_use]
    pub const fn owner(&self) -> u32 {
        self.owner
    }

    /// Returns the group identity that must own the socket directory and node.
    #[must_use]
    pub const fn group(&self) -> u32 {
        self.group
    }

    /// Returns whether this peer is admitted to the socket at all.
    #[must_use]
    pub fn admits(&self, peer: &PeerIdentity) -> bool {
        self.lifecycle.contains(&peer.uid) || self.reconcile.contains(&peer.uid)
    }

    /// Returns whether this peer may request this capability.
    #[must_use]
    pub fn permits(&self, peer: &PeerIdentity, capability: Capability) -> bool {
        match capability {
            Capability::Lifecycle => self.lifecycle.contains(&peer.uid),
            Capability::Reconcile => self.reconcile.contains(&peer.uid),
        }
    }
}

#[cfg(test)]
mod tests;
