//! The exact Launch frame this Backend registers one Instance with.
//!
//! Every value in the frame is derived from the request the Backend was given, so a replay of
//! one operation presents byte-for-byte the same frame and the Host answers it as the replay
//! it is rather than as a changed intent.

use std::time::Duration;

use soma::{BackendFailureKind, InstanceId, OperationId};
use soma_hostd::{
    InstanceId as HostInstance, LaunchFrame, LaunchMaterialHandle, OperationId as HostOperation,
};
use soma_netd::{EgressClass, NetworkIntent, ProfileDigest};

use crate::backend::kvm::identity::hex16;

/// How long a registered Instance is declared to live.
///
/// The Host delivers this to the worker as the Instance deadline. This Backend has no lifetime
/// of its own to declare, because its sandbox lives until the invocation that owns it cleans
/// it up, so the declared value is the one bound the Backend does enforce: the ceiling of a
/// single bounded command. A larger number would be an assertion nothing here keeps.
pub(in crate::backend::kvm) const DECLARED_LIFETIME: Duration =
    crate::backend::kvm::lifecycle::COMMAND_CEILING;

/// The Instance identity as the Host names it.
///
/// # Errors
///
/// Returns [`BackendFailureKind::WorkloadRejected`] when the portable identity is not the
/// thirty-two hexadecimal characters both sides define it to be.
pub(super) fn host_instance(instance: &InstanceId) -> Result<HostInstance, BackendFailureKind> {
    HostInstance::new(hex16(instance.as_str())?).map_err(|_| BackendFailureKind::WorkloadRejected)
}

/// The exact frame that registers `instance` with the Host Runtime.
///
/// # Errors
///
/// Returns [`BackendFailureKind::WorkloadRejected`] when an identity cannot be carried on the
/// wire, which is a property of the request rather than of the Host.
pub(super) fn launch_frame(
    instance: &InstanceId,
    operation: &OperationId,
    vsock_cid: u32,
) -> Result<LaunchFrame, BackendFailureKind> {
    let instance_bytes = hex16(instance.as_str())?;
    let operation_bytes = hex16(operation.as_str())?;
    Ok(LaunchFrame {
        operation: HostOperation::new(operation_bytes)
            .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        instance: HostInstance::new(instance_bytes)
            .map_err(|_| BackendFailureKind::WorkloadRejected)?,
        vsock_cid,
        deadline_nanos: u64::try_from(DECLARED_LIFETIME.as_nanos()).unwrap_or(u64::MAX),
        launch_material: material_handle(instance_bytes, operation_bytes)?,
        intent: denied_intent()?,
    })
}

/// The handle this launch's sealed material is named by.
///
/// A Host that seals launch material hands out a handle for it. This Backend seals its own
/// material inside the sandbox thread and has no host-sealed blob to name, so the handle is
/// the Instance and operation identities laid end to end: it names exactly this launch, it is
/// the same on every replay of the operation, and it claims nothing about a sealing that did
/// not happen.
fn material_handle(
    instance: [u8; 16],
    operation: [u8; 16],
) -> Result<LaunchMaterialHandle, BackendFailureKind> {
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(&instance);
    bytes[16..].copy_from_slice(&operation);
    LaunchMaterialHandle::new(bytes).map_err(|_| BackendFailureKind::WorkloadRejected)
}

/// The network intent that matches the machine this Backend actually builds.
///
/// The guest's one device is link down, so egress is denied and no resolver is offered. The
/// profile digest is zero because no served network profile backs this device: naming one
/// would claim an admission against a profile that was never consulted.
fn denied_intent() -> Result<NetworkIntent, BackendFailureKind> {
    NetworkIntent::new(
        EgressClass::Denied,
        Vec::new(),
        Vec::new(),
        ProfileDigest([0; 32]),
    )
    .map_err(|_| BackendFailureKind::WorkloadRejected)
}
