//! Build a worker's machine in a fresh jail when none was prepared for it.

use super::{
    Anchors, BackendFailure, BackendFailureKind, Egress, Held, Jailed, JailedLaunching, KvmBackend,
    Launching, NetworkLaunch, OperationId, PreparationClass, Started, claim, generation_bytes,
};

impl KvmBackend {
    pub(super) fn build_in_a_jail(
        &mut self,
        operation: &OperationId,
        launching: Launching<'_>,
        egress: &mut Egress,
        anchors: &Anchors,
    ) -> Result<Started, BackendFailure> {
        let Launching {
            instance,
            prepared,
            identity,
            memory_mib,
            storage_mib,
            network,
            secrets,
            volume,
            checkpoint,
            // Never set here: a resume claims nothing and is refused in a jail.
            resume: _,
        } = launching;
        if !secrets.is_empty() {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        let network = match (network.attachment, network.activation) {
            (None, None) => None,
            (Some(attachment), Some(pending)) if attachment.mac == network.launch.mac() => Some((
                attachment.tap,
                NetworkLaunch {
                    launch: network.launch,
                    pending,
                    operation: identity.operation,
                    egress,
                },
            )),
            _ => return Err(self.fail(operation, BackendFailureKind::Unsupported)),
        };
        // Only a Generation with a captured machine can be restored from descriptors; a cold
        // boot would need the kernel and initramfs this descriptor table has no roles for.
        if claim::snapshot(prepared).is_none() {
            return Err(self.fail(operation, BackendFailureKind::Unsupported));
        }
        let generation_bytes =
            generation_bytes(&prepared.id).map_err(|kind| self.fail(operation, kind))?;
        let restoring_in_place = checkpoint.is_some_and(|(source, _)| source == instance);
        self.ownership
            .register_jailed(instance, operation, identity, restoring_in_place)
            .map_err(|kind| self.fail(operation, kind))?;
        let launched = self.clocks.elapsed_ns(operation);
        match Jailed::launch(
            anchors,
            &JailedLaunching {
                operation,
                prepared,
                instance,
                instance_bytes: identity.instance,
                generation_bytes,
                memory_mib,
                disk_mib: storage_mib,
                checkpoint,
                volume: volume.as_ref(),
            },
            network,
        ) {
            Ok(jailed) => Ok(Started {
                preparation: PreparationClass::OnDemand,
                held: Held::Jailed(Box::new(jailed)),
                launched,
                // A jailed machine has no private head of its own to warm.
                head_reads: Vec::new(),
            }),
            Err(kind) => {
                // A launch that never produced a machine leaves no Instance owned by this Host.
                self.ownership.withdraw(instance);
                Err(BackendFailure::new(kind, self.clocks.elapsed_ns(operation)))
            }
        }
    }
}
