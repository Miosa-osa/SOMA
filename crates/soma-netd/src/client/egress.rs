//! Changing the egress of a bundle the caller already holds.

use crate::{BundleId, CleanupGeneration, EgressClass, InstanceId, Reply, Request};

use super::{BrokerClient, ClientError};

/// The egress state a bundle is in after [`BrokerClient::set_egress`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EgressState {
    /// Whether guest egress is permitted.
    pub open: bool,
    /// Whether DNS to the profile's resolvers is permitted.
    pub dns: bool,
    /// Whether the ruleset differed from before the call.
    pub changed: bool,
}

impl BrokerClient {
    /// Sets the egress of an activated bundle this peer claimed, without restarting its guest.
    ///
    /// Returns the state now in force. `egress` is `Denied` or `PublicInternet`; `dns` allows the
    /// profile's resolvers and is refused with `Denied`. Closing flushes the bundle's tracked
    /// connections, so none survives the close.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError::Refused`] with the broker's code for a bundle that is not the
    /// caller's, an Instance that does not match, a bundle that is not activated, or a kernel or
    /// tool failure; the caller must then treat egress as open. A reply that does not describe
    /// this assignment is [`ClientError::Protocol`].
    pub fn set_egress(
        &self,
        instance: InstanceId,
        bundle: BundleId,
        generation: CleanupGeneration,
        egress: EgressClass,
        dns: bool,
    ) -> Result<EgressState, ClientError> {
        let request = Request::SetEgress {
            instance,
            bundle,
            generation,
            egress,
            dns,
        };
        match self.call(&request)? {
            (
                Reply::EgressSet {
                    bundle: replied,
                    generation: replied_generation,
                    open,
                    dns,
                    changed,
                },
                None,
            ) if replied == bundle && replied_generation == generation => {
                Ok(EgressState { open, dns, changed })
            }
            _ => Err(ClientError::Protocol),
        }
    }
}
