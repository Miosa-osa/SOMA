//! The network a restored machine gets from its vsock CID.

use super::{ActivationChannel, File, Network, NetworkAttachment, SessionError, link_down_network};

pub(super) fn network_for_cid(
    resources: Option<(File, ActivationChannel)>,
    guest_cid: u32,
) -> Result<(Network, Option<ActivationChannel>), SessionError> {
    let Some((tap, channel)) = resources else {
        let launch = link_down_network(guest_cid).ok_or(SessionError::Create)?;
        return Ok((
            Network {
                launch,
                attachment: None,
                activation: None,
            },
            None,
        ));
    };
    let (launch, pending) = channel.read_offer()?;
    if launch.vsock_cid() != guest_cid {
        return Err(SessionError::Network);
    }
    let attachment = NetworkAttachment {
        tap,
        mac: launch.mac(),
    };
    Ok((
        Network {
            launch,
            attachment: Some(attachment),
            activation: Some(pending),
        },
        Some(channel),
    ))
}
