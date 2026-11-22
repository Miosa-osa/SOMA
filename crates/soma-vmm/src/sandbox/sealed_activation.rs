//! The one exchange a sealed worker has with the supervisor that owns its network lease.
//!
//! The worker receives only the launch identity and a single-use challenge. It sends the receipt
//! minted after guest authentication back to the supervisor, which can ask the network broker to
//! activate the lease. The worker raises the guest's link only after that succeeds.

use std::time::Duration;

use soma_guest::{ActivationChallenge, ActivationReceipt, LaunchNetwork};
use soma_jail::ControlSocket;

use super::{Activation, PendingActivation, SessionError};

const NETWORK_BYTES: usize = LaunchNetwork::WIRE_SIZE;
const CHALLENGE_END: usize = NETWORK_BYTES + 32;
const OFFER_BYTES: usize = CHALLENGE_END + 32;
const BROKER_WAIT: Duration = Duration::from_secs(10);

/// A one-launch connected socket. No broker socket or network namespace handle enters the worker.
pub struct ActivationChannel(ControlSocket);

impl ActivationChannel {
    /// Adopts the local end of the connected activation socket.
    #[must_use]
    pub const fn new(socket: ControlSocket) -> Self {
        Self(socket)
    }

    /// Sends the lease's identity and activation challenge to the worker.
    ///
    /// # Errors
    /// Returns [`SessionError::Network`] if the arguments disagree or the worker is gone.
    pub fn offer(
        &self,
        network: LaunchNetwork,
        pending: &PendingActivation,
    ) -> Result<(), SessionError> {
        if network.generation() != pending.generation {
            return Err(SessionError::Network);
        }
        let mut packet = [0_u8; OFFER_BYTES];
        packet[..NETWORK_BYTES].copy_from_slice(&network.to_bytes());
        packet[NETWORK_BYTES..CHALLENGE_END].copy_from_slice(&pending.challenge.to_bytes());
        packet[CHALLENGE_END..].copy_from_slice(&pending.intent);
        let result = self
            .0
            .send_bytes(&packet)
            .map_err(|_| SessionError::Network);
        packet.fill(0);
        result
    }

    pub(crate) fn read_offer(&self) -> Result<(LaunchNetwork, PendingActivation), SessionError> {
        let mut packet = [0_u8; OFFER_BYTES];
        if self.0.receive_into(&mut packet, BROKER_WAIT) != Ok(OFFER_BYTES) {
            return Err(SessionError::Network);
        }
        let network = LaunchNetwork::from_bytes(
            packet[..NETWORK_BYTES]
                .try_into()
                .map_err(|_| SessionError::Network)?,
        )
        .map_err(|_| SessionError::Network)?;
        let challenge = ActivationChallenge::from_bytes(
            packet[NETWORK_BYTES..CHALLENGE_END]
                .try_into()
                .map_err(|_| SessionError::Network)?,
        )
        .map_err(|_| SessionError::Network)?;
        let intent = packet[CHALLENGE_END..]
            .try_into()
            .map_err(|_| SessionError::Network)?;
        packet.fill(0);
        Ok((
            network,
            PendingActivation {
                challenge,
                generation: network.generation(),
                intent,
            },
        ))
    }

    /// Verifies that the worker sent a well-formed receipt and forwards it to the lease owner.
    ///
    /// # Errors
    /// Returns [`SessionError::Network`] if the exchange fails or the lease owner refuses it.
    pub fn confirm(
        &self,
        within: Duration,
        activate: impl FnOnce(&ActivationReceipt) -> bool,
    ) -> Result<(), SessionError> {
        let mut bytes = [0_u8; ActivationReceipt::LEN];
        if self.0.receive_into(&mut bytes, within) != Ok(bytes.len()) {
            return Err(SessionError::Network);
        }
        let receipt = ActivationReceipt::from_bytes(&bytes).map_err(|_| SessionError::Network)?;
        if !activate(&receipt) {
            return Err(SessionError::Network);
        }
        self.0.send_bytes(&[1]).map_err(|_| SessionError::Network)
    }

    pub(crate) fn ask(&self, receipt: &ActivationReceipt) -> Result<Activation, SessionError> {
        self.0
            .send_bytes(&receipt.to_bytes())
            .map_err(|_| SessionError::Network)?;
        let mut reply = [0];
        if self.0.receive_into(&mut reply, BROKER_WAIT) != Ok(1) || reply != [1] {
            return Err(SessionError::Network);
        }
        Ok(Activation::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_receipt_the_lease_owner_accepts_releases_the_worker() {
        let (broker, worker) = ControlSocket::pair().expect("socket pair");
        let channel = ActivationChannel::new(broker);
        let worker = ControlSocket::adopt(worker);
        let receipt = [7; ActivationReceipt::LEN];
        worker.send_bytes(&receipt).expect("receipt sent");
        assert!(channel.confirm(Duration::ZERO, |_| false).is_err());
        let mut reply = [0];
        assert!(worker.receive_into(&mut reply, Duration::ZERO).is_err());

        worker.send_bytes(&receipt).expect("receipt sent");
        channel
            .confirm(Duration::ZERO, |candidate| candidate.to_bytes() == receipt)
            .expect("lease owner accepted the receipt");
        assert_eq!(worker.receive_into(&mut reply, Duration::ZERO), Ok(1));
        assert_eq!(reply, [1]);
    }

    #[test]
    fn an_oversized_receipt_is_refused_before_reaching_the_lease_owner() {
        let (broker, worker) = ControlSocket::pair().expect("socket pair");
        let channel = ActivationChannel::new(broker);
        let worker = ControlSocket::adopt(worker);
        worker
            .send_bytes(&[7; ActivationReceipt::LEN + 1])
            .expect("packet sent");
        assert!(
            channel
                .confirm(Duration::ZERO, |_| panic!(
                    "oversized packet escaped the decoder"
                ))
                .is_err()
        );
    }
}
