//! Changing the egress of the Instance a lease belongs to.

use soma_netd::{BrokerClient, ClientError, EgressClass, Reply, Request};

use crate::{EgressSetFailure, EgressSetOutcome, EgressSetRequest};

use super::{ACTIVATION_PATIENCE, Activation, Lease};

impl Lease {
    /// Opens or closes this Instance's egress through the broker and reports what is in force.
    ///
    /// The broker answers only once the rules are in the kernel, and a close has flushed the
    /// tracked flows by then, so an outcome is a statement about the network, not a promise.
    pub(in crate::backend::kvm) fn set_egress(
        &mut self,
        wanted: EgressSetRequest,
    ) -> Result<EgressSetOutcome, EgressSetFailure> {
        // The rules of a bundle that is not activated cannot be changed, and an activation that
        // is still in flight is waited for, as a dial waits for it.
        if matches!(
            self.settled.wait(ACTIVATION_PATIENCE),
            Activation::Running | Activation::Refused
        ) {
            return Err(EgressSetFailure::NotActive);
        }
        let request = Request::SetEgress {
            instance: self.instance,
            bundle: self.bundle,
            generation: self.generation,
            egress: if wanted.open {
                EgressClass::PublicInternet
            } else {
                EgressClass::Denied
            },
            dns: wanted.dns,
        };
        // A closed idle connection is reconnected once: the request is idempotent under the
        // bundle and generation, and the broker authenticates the peer by its kernel identity.
        let answer = match self.connection.call(&request) {
            Err(ClientError::Unreachable | ClientError::Protocol) => {
                self.connection = BrokerClient::connect(&self.socket)
                    .map_err(|_| EgressSetFailure::Unavailable)?;
                self.connection.call(&request)
            }
            other => other,
        };
        match answer {
            Ok((
                Reply::EgressSet {
                    open, dns, changed, ..
                },
                None,
            )) => Ok(EgressSetOutcome { open, dns, changed }),
            Err(ClientError::Refused(code)) => Err(refusal(code)),
            // A reply that is not the egress answer, or no answer at all.
            Ok(_) | Err(_) => Err(EgressSetFailure::Unavailable),
        }
    }
}

/// What a broker refusal of an egress change means to the caller: 11 and 16 are a bundle that is
/// not (or no longer) activated, 18 is an authorization refusal, and everything else is a kernel
/// or tool failure while applying the rules.
const fn refusal(code: u16) -> EgressSetFailure {
    match code {
        11 | 16 => EgressSetFailure::NotActive,
        18 => EgressSetFailure::Unavailable,
        _ => EgressSetFailure::FlipFailed,
    }
}
