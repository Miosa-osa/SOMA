//! Exact projection of a network envelope onto the portable `NetworkPolicy`.
//!
//! The portable contract has no destination-filtered egress class and no unbounded ingress,
//! so only the envelopes it can state exactly are mapped; every other envelope fails closed
//! rather than being widened or silently narrowed.

use soma::{DnsPolicy, EgressPolicy, NetworkPolicy};

use super::RevisionError;
use crate::{
    schema::IngressIntent,
    validate::{EgressEnvelope, NetworkEnvelope},
};

pub(super) fn policy(envelope: &NetworkEnvelope) -> Result<NetworkPolicy, RevisionError> {
    if envelope.ingress() != IngressIntent::Deny {
        return Err(RevisionError::UnrepresentableNetwork);
    }
    match envelope.egress() {
        EgressEnvelope::Deny => Ok(NetworkPolicy::isolated()),
        EgressEnvelope::Unrestricted => {
            NetworkPolicy::new(EgressPolicy::Unrestricted, DnsPolicy::System, Vec::new())
                .map_err(|_| RevisionError::UnrepresentableNetwork)
        }
        EgressEnvelope::Allowlist { .. } => Err(RevisionError::UnrepresentableNetwork),
    }
}
