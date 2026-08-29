//! The eight ordered authority frames and the host entropy they carry.

use crate::{
    AssignedResources, AssignmentIntent, LeaseGeneration, TransferFault, TransferFrame, WorkerId,
};

pub(super) fn frames(
    worker: WorkerId,
    lease_generation: LeaseGeneration,
    intent: &AssignmentIntent,
    seed: [u8; 32],
    resources: AssignedResources,
) -> [TransferFrame; 8] {
    let AssignedResources {
        disk,
        network,
        control,
    } = resources;
    [
        TransferFrame::Identity {
            worker,
            lease_generation,
            instance: intent.instance,
            operation: intent.operation,
        },
        TransferFrame::Deadline {
            deadline_nanos: intent.deadline_nanos(),
        },
        TransferFrame::Entropy { seed },
        TransferFrame::LaunchPage {
            material: intent.launch_material,
            network: network.launch,
        },
        TransferFrame::Disk(disk.head),
        TransferFrame::Network(network.tap),
        TransferFrame::Control {
            vsock_cid: control.vsock_cid,
            channel: control.channel,
        },
        TransferFrame::Commit,
    ]
}

#[cfg(unix)]
pub(super) fn fresh_entropy() -> Result<[u8; 32], TransferFault> {
    use std::io::Read;
    let mut seed = [0; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut seed))
        .map_err(|_| TransferFault::Entropy)?;
    if seed.iter().all(|byte| *byte == 0) {
        return Err(TransferFault::Entropy);
    }
    Ok(seed)
}

#[cfg(not(unix))]
pub(super) fn fresh_entropy() -> Result<[u8; 32], TransferFault> {
    Err(TransferFault::Entropy)
}
