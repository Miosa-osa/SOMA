//! The network one Instance is given, from claim to release.
//!
//! A guest that can reach nothing and a guest whose network failed must never look alike, so
//! this has exactly two outcomes and no third. Either the request asked for no egress and the
//! Instance keeps the link-down placeholder device, which is stated as a fact rather than
//! implied by absence; or the privileged broker leased this Instance a real bundle and every
//! later step names that lease. A request that asked for egress on a host with no reachable
//! broker is refused, because serving it from the placeholder would report a working network
//! that drops every packet.
//!
//! The lease outlives nothing. It is released when cleanup releases it, and released again by
//! [`Drop`] on every path that never reached cleanup, because a namespace, TAP, address lease,
//! or port mapping that outlives its Instance is the failure that compounds fastest.

mod intent;
mod lease;

use soma::{BackendFailureKind, EgressPolicy, NetworkPolicy};
use soma_guest::{ActivationChallenge, ActivationReceipt, LaunchNetwork};
use soma_kvm::x86_64::NetworkAttachment;
use soma_netd::ClientError;

pub(super) use self::intent::BrokerConfiguration;
pub(super) use self::lease::Lease;
use super::identity::LaunchIdentity;

/// The network one Instance holds.
pub(super) enum Egress {
    /// The request asked for no egress, so the guest keeps the device it was built with.
    ///
    /// The device drops every frame while its link is down, which is a machine with no network
    /// rather than a machine whose network failed.
    Declined,
    /// The broker leased this Instance one sterile bundle.
    Leased(Box<Lease>),
}

impl Egress {
    /// Obtains the network this request asked for.
    ///
    /// # Errors
    ///
    /// Returns [`BackendFailureKind::Unsupported`] when the request needs egress and this host
    /// has no reachable broker, and [`BackendFailureKind::WorkloadRejected`] when the broker
    /// refuses the stated policy.
    pub(super) fn claim(
        configuration: Option<&BrokerConfiguration>,
        policy: &NetworkPolicy,
        identity: LaunchIdentity,
    ) -> Result<Self, BackendFailureKind> {
        if matches!(
            policy.egress(),
            EgressPolicy::Denied | EgressPolicy::Unspecified
        ) {
            return Ok(Self::Declined);
        }
        // Saying so is the whole point of a fail-closed network: a host with no broker cannot
        // serve egress, and reporting otherwise would describe a machine that reaches nothing.
        let configuration = configuration.ok_or(BackendFailureKind::Unsupported)?;
        let intent = configuration
            .admit(policy)
            .ok_or(BackendFailureKind::WorkloadRejected)?;
        Lease::claim(&configuration.socket, &intent, identity)
            .map(|lease| Self::Leased(Box::new(lease)))
    }

    /// The launch-page network values this Instance is given.
    pub(super) fn launch(
        &self,
        identity: LaunchIdentity,
    ) -> Result<LaunchNetwork, BackendFailureKind> {
        match self {
            Self::Declined => super::boot::link_down_network(identity.guest_cid),
            Self::Leased(lease) => Ok(lease.launch()),
        }
    }

    /// The frame path the machine attaches, when there is one.
    pub(super) fn attachment(&mut self) -> Option<NetworkAttachment> {
        match self {
            Self::Declined => None,
            Self::Leased(lease) => lease.attachment(),
        }
    }

    /// What the guest must present before the broker will let traffic flow.
    pub(super) fn pending_activation(&self) -> Option<PendingActivation> {
        match self {
            Self::Declined => None,
            Self::Leased(lease) => Some(lease.pending_activation()),
        }
    }

    /// Activates the lease with the receipt the repaired guest session minted.
    ///
    /// # Errors
    ///
    /// Returns the broker's refusal; the lease is released before the failure is returned, so a
    /// launch that cannot activate leaves nothing behind.
    pub(super) fn activate(&mut self, receipt: &ActivationReceipt) -> Result<(), ()> {
        match self {
            Self::Declined => Ok(()),
            Self::Leased(lease) => lease.activate(receipt),
        }
    }

    /// Releases whatever this Instance held, and reports whether the broker verified it gone.
    pub(super) fn release(&mut self) -> Released {
        match self {
            Self::Declined => Released::NothingHeld,
            Self::Leased(lease) => lease.release(),
        }
    }
}

/// What a release established about the resources the broker held.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Released {
    /// This Instance never held a network resource.
    NothingHeld,
    /// The broker released everything and its live inspection found nothing owned.
    Complete,
    /// The broker was asked but could not confirm; reconciliation owns what is left.
    Incomplete,
}

/// The single-use capability the repaired guest session must mint.
pub(super) struct PendingActivation {
    /// The broker's fresh secret for this assignment.
    pub(super) challenge: ActivationChallenge,
    /// The assignment generation the receipt is bound to.
    pub(super) generation: u32,
    /// The digest of the admitted intent the receipt is bound to.
    pub(super) intent: [u8; 32],
}

/// The typed failure one broker refusal becomes.
const fn kind_for(error: ClientError) -> BackendFailureKind {
    match error {
        // No broker is the same condition as no broker configured: an operator fact.
        ClientError::Unreachable => BackendFailureKind::Unsupported,
        // The broker answered, so the host is capable; the exchange did not complete.
        ClientError::Protocol => BackendFailureKind::Unavailable,
        // The broker refused what this request asked for.
        ClientError::Refused(_) => BackendFailureKind::WorkloadRejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> LaunchIdentity {
        let instance = soma::InstanceId::new("89db112753324c3e890ef78b74381aa5").expect("instance");
        LaunchIdentity::derive(&instance).expect("identity")
    }

    /// A request that asked for nothing is answered with nothing, not with a lease attempt.
    #[test]
    fn a_request_for_no_egress_never_reaches_a_broker() {
        let identity = identity();
        for policy in [NetworkPolicy::isolated(), NetworkPolicy::runtime_default()] {
            let mut egress = Egress::claim(None, &policy, identity).expect("no egress requested");
            assert!(matches!(egress, Egress::Declined));
            assert!(egress.attachment().is_none());
            assert!(egress.pending_activation().is_none());
            assert_eq!(egress.release(), Released::NothingHeld);
            // The guest still repairs an interface, so it is still given launch values, and they
            // name the identifier this machine was built with.
            let launch = egress.launch(identity).expect("link-down values");
            assert_eq!(launch.vsock_cid(), identity.guest_cid);
        }
    }

    /// A request that needs egress on a host with no broker is refused, never quietly served.
    #[test]
    fn egress_without_a_broker_is_unsupported_rather_than_a_placeholder() {
        let policy = NetworkPolicy::new(
            EgressPolicy::PublicInternet,
            soma::DnsPolicy::System,
            Vec::new(),
        )
        .expect("policy");
        assert_eq!(
            Egress::claim(None, &policy, identity()).err(),
            Some(BackendFailureKind::Unsupported)
        );
    }
}
