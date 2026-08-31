//! One network assignment, held for the life of the Instance it was leased to.

use soma::BackendFailureKind;
use soma_guest::{ActivationReceipt, LaunchNetwork};
use soma_kvm::x86_64::NetworkAttachment;
use soma_netd::{
    BrokerClient, BundleId, CleanupGeneration, InstanceId, NetworkIntent, OperationId, Reply,
    Request,
};

use super::super::identity::LaunchIdentity;
use super::{PendingActivation, Released, kind_for};

/// One assignment held for the life of one Instance.
pub(in crate::backend::kvm) struct Lease {
    connection: BrokerClient,
    bundle: BundleId,
    generation: CleanupGeneration,
    launch: LaunchNetwork,
    activation: PendingActivation,
    /// Taken exactly once, by the machine that attaches it.
    tap: Option<std::fs::File>,
    released: bool,
}

impl Lease {
    /// Claims one bundle from the broker listening at `socket`.
    pub(super) fn claim(
        socket: &std::path::Path,
        intent: &NetworkIntent,
        identity: LaunchIdentity,
    ) -> Result<Self, BackendFailureKind> {
        // An unreachable broker is the same condition as an unconfigured one.
        let connection = BrokerClient::connect(socket).map_err(kind_for)?;
        let request = Request::Claim {
            instance: InstanceId::new(identity.instance)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            operation: OperationId::new(identity.operation)
                .map_err(|_| BackendFailureKind::WorkloadRejected)?,
            vsock_cid: identity.guest_cid,
            intent: intent.clone(),
        };
        let (reply, descriptor) = connection.call(&request).map_err(kind_for)?;
        let Reply::Claimed {
            bundle,
            generation,
            launch,
            activation,
        } = reply
        else {
            return Err(BackendFailureKind::Unavailable);
        };
        // A claim the broker answered without transferring the frame path would leave a lease
        // no machine can use, so it is refused here and released by the drop below.
        let tap = descriptor.ok_or(BackendFailureKind::Unavailable)?;
        let launch = decode_launch(&launch).ok_or(BackendFailureKind::Unavailable)?;
        Ok(Self {
            connection,
            bundle,
            generation,
            launch,
            activation: PendingActivation {
                challenge: activation,
                generation: generation.get(),
                intent: intent.digest().0,
            },
            tap: Some(std::fs::File::from(tap)),
            released: false,
        })
    }

    pub(super) fn attachment(&mut self) -> Option<NetworkAttachment> {
        self.tap.take().map(|tap| NetworkAttachment {
            tap,
            mac: self.launch.mac(),
        })
    }

    /// The address the broker leased this Instance, as portable evidence.
    pub(in crate::backend::kvm) fn addresses(&self) -> Vec<soma::AssignedAddress> {
        soma::AssignedAddress::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::from(self.launch.address())),
            self.launch.prefix_length(),
        )
        .map_or_else(|_| Vec::new(), |address| vec![address])
    }

    /// The launch-page network values the broker leased.
    pub(super) const fn launch(&self) -> LaunchNetwork {
        self.launch
    }

    pub(super) fn pending_activation(&self) -> PendingActivation {
        PendingActivation {
            challenge: self.activation.challenge.clone(),
            generation: self.activation.generation,
            intent: self.activation.intent,
        }
    }

    pub(super) fn activate(&mut self, receipt: &ActivationReceipt) -> Result<(), ()> {
        let request = Request::Activate {
            bundle: self.bundle,
            generation: self.generation,
            receipt: *receipt,
        };
        if let Ok((Reply::Activated, _)) = self.connection.call(&request) {
            return Ok(());
        }
        // A refused activation leaves a bundle the broker may already have torn down, and in
        // every case one this Instance must not keep holding.
        let _released = self.release();
        Err(())
    }

    pub(super) fn release(&mut self) -> Released {
        if self.released {
            return Released::Complete;
        }
        self.released = true;
        // The descriptor is closed before the request, so the broker's own inspection cannot
        // find this process still holding the TAP it is trying to release.
        drop(self.tap.take());
        let request = Request::Release {
            bundle: self.bundle,
            generation: self.generation,
        };
        match self.connection.call(&request) {
            Ok((Reply::Released { complete: true }, _)) => Released::Complete,
            _ => Released::Incomplete,
        }
    }
}

impl Drop for Lease {
    /// A lease must not outlive the Instance that holds it.
    ///
    /// Every failure path between claim and cleanup ends here: a machine that could not be
    /// built, a guest that never reached its session, an activation the broker refused, or a
    /// panic. Releasing here is what keeps a namespace, TAP, address lease, and port mapping
    /// from surviving the sandbox that owned them.
    fn drop(&mut self) {
        let _released = self.release();
    }
}

/// Rebuilds the launch-page network values from the exact bytes the broker replied with.
fn decode_launch(bytes: &[u8; 35]) -> Option<LaunchNetwork> {
    let word =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let quad = |at: usize| [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]];
    let mut mac = [0_u8; 6];
    mac.copy_from_slice(&bytes[8..14]);
    let mut sample = [0_u8; 8];
    sample.copy_from_slice(&bytes[27..35]);
    LaunchNetwork::new(
        word(0),
        word(4),
        mac,
        quad(14),
        bytes[18],
        quad(19),
        quad(23),
        u64::from_be_bytes(sample),
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The broker's reply is the launch page, so the two must carry the same values.
    ///
    /// The reply carries the fields as fixed bytes rather than as a structure, so an offset that
    /// drifted would hand the guest a plausible address belonging to no lease. Encoding what the
    /// broker encodes and decoding it back is the only check that catches that.
    #[test]
    fn the_decoded_launch_values_are_the_ones_the_broker_encoded() {
        let expected = LaunchNetwork::new(
            9,
            3,
            [0x02, 1, 2, 3, 4, 5],
            [10, 200, 0, 6],
            30,
            [10, 200, 0, 5],
            [1, 1, 1, 1],
            0x0102_0304_0506_0708,
        )
        .expect("launch values");
        let mut bytes = [0_u8; 35];
        bytes[..4].copy_from_slice(&expected.vsock_cid().to_be_bytes());
        bytes[4..8].copy_from_slice(&expected.generation().to_be_bytes());
        bytes[8..14].copy_from_slice(&expected.mac());
        bytes[14..18].copy_from_slice(&expected.address());
        bytes[18] = expected.prefix_length();
        bytes[19..23].copy_from_slice(&expected.gateway());
        bytes[23..27].copy_from_slice(&expected.resolver());
        bytes[27..35].copy_from_slice(&expected.time_sample_nanos().to_be_bytes());

        let decoded = decode_launch(&bytes).expect("decodes");
        assert_eq!(decoded.vsock_cid(), expected.vsock_cid());
        assert_eq!(decoded.generation(), expected.generation());
        assert_eq!(decoded.mac(), expected.mac());
        assert_eq!(decoded.address(), expected.address());
        assert_eq!(decoded.prefix_length(), expected.prefix_length());
        assert_eq!(decoded.gateway(), expected.gateway());
        assert_eq!(decoded.resolver(), expected.resolver());
        assert_eq!(decoded.time_sample_nanos(), expected.time_sample_nanos());
    }

    /// A reply no broker could have produced must never become a launch page.
    #[test]
    fn launch_values_no_broker_could_have_leased_are_refused() {
        assert!(decode_launch(&[0_u8; 35]).is_none());
    }
}
