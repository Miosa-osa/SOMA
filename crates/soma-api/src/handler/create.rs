//! Create a sandbox and answer a lifecycle change.

use super::{
    ApiError, CreateSandboxBody, Envelope, ExecutionReceipt, MissingCapability, Response,
    SandboxFacade, SandboxReport, VOLUME_DEVICE, VolumeReady, managed_error, parse, render,
    success, volume_binding,
};

pub(super) fn create<F: SandboxFacade + ?Sized>(
    facade: &mut F,
    body: &[u8],
) -> Result<Response, ApiError> {
    let body = parse::<CreateSandboxBody>(body)?;
    let named_volume = body.volume_id.clone();
    let (instance_id, request) = body.into_facade()?;
    // A start that names its volume binds that file and no other: the host resolves it from the
    // volume this Instance owns, and a disagreement between the two is refused rather than
    // resolved by guessing which is right.
    let (request, volume, fsck) = match named_volume.as_deref() {
        Some(named) => {
            let (attachment, fsck) = volume_binding(facade, &instance_id, named)?;
            (
                request.with_volume(attachment),
                Some(VolumeReady {
                    volume_id: named.to_ascii_lowercase(),
                    state: "attached",
                    device: VOLUME_DEVICE,
                }),
                Some(fsck),
            )
        }
        None => (request, None, None),
    };
    // Refused before a machine is built rather than answered 201 with an identity that dies
    // with this connection. A caller that keeps the identity and returns is the whole point of
    // the create route, and there would be nothing here for it to return to.
    if !facade.hosts_addressable_sandboxes() {
        return Err(MissingCapability::DurableMachineHosting.error());
    }
    let outcome = facade.launch(request).map_err(|f| managed_error(&f))?;
    // A created sandbox answers 201, matching what a provider contract expects of a create call
    // that produced a new addressable resource.
    Ok(Response::new(
        201,
        render(&Envelope::success(
            "sandbox.create",
            &SandboxReport {
                instance_id: outcome.instance_id,
                state: "ready",
                volume,
                fsck,
            },
            Some(&outcome.receipt),
        )),
    ))
}

pub(super) fn lifecycle(
    operation: &'static str,
    state: &'static str,
    instance_id: &soma::InstanceId,
    receipt: &ExecutionReceipt,
) -> Response {
    success(
        200,
        operation,
        &SandboxReport::new(instance_id.clone(), state),
        Some(receipt),
    )
}
