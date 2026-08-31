//! Assembling a sandbox from restored state instead of from a cold boot.
//!
//! Everything after this point - the control channel, the fresh launch page, the repair
//! commit, the milestones, and the ordered teardown - is the same code the cold-boot proof
//! uses, so a restored Instance cannot take a weaker path to `Ready` than a booted one.

use std::sync::{Arc, Mutex, atomic::AtomicBool};

use kvm_ioctls::VcpuFd;
use vmm_sys_util::eventfd::EventFd;

use super::{Prepared, SandboxMachine, Stage, Timeline};
use crate::virtio::{FileBackend, MmioBus, VsockDevice};
use crate::x86_64::{
    Machine,
    devices::SharedBus,
    error::{MachineError, MachineErrorKind, Phase},
    events::{IrqLines, NotifyFds},
    launch_page::LaunchPageSlot,
    timing::Stopwatch,
};

/// Every owned resource a restore produced, in the order the machine will release them.
pub(in crate::x86_64) struct RestoredParts {
    pub(in crate::x86_64) machine: Machine,
    pub(in crate::x86_64) bus: MmioBus,
    pub(in crate::x86_64) vcpu: VcpuFd,
    pub(in crate::x86_64) serial_line: EventFd,
    pub(in crate::x86_64) irq: IrqLines,
    pub(in crate::x86_64) notify: NotifyFds,
    pub(in crate::x86_64) launch_page: LaunchPageSlot,
    pub(in crate::x86_64) clock: Stopwatch,
    pub(in crate::x86_64) timeline: Timeline,
    /// The exact command line the captured Generation was booted with.
    pub(in crate::x86_64) cmdline: String,
}

/// The frame path and address identity of one assigned network bundle.
///
/// A bundle is per-Instance authority, so a prepared worker cannot hold one and its network
/// device is built without a frame path. This is what arrives when the worker is claimed.
///
/// The descriptor must already be non-blocking: the device thread reads it inline, so a blocking
/// descriptor would stall every other device on the bus.
pub struct NetworkAttachment {
    /// The prepared TAP descriptor from the assigned bundle.
    pub tap: std::fs::File,
    /// The MAC the bundle leased for this Instance.
    pub mac: [u8; 6],
}

impl SandboxMachine {
    /// Gives a machine built without them the private disk head and context identifier.
    ///
    /// A prepared worker is restored before any Instance exists, so it is built against a
    /// declared overlay shape and the identifier the snapshot was captured holding. Both are
    /// per-Instance authority the prepared worker protocol transfers when the worker is claimed,
    /// and this is where that transfer lands in the machine.
    ///
    /// It is safe to do this to an already restored device because the vCPU has not been started:
    /// nothing in the guest has read the identifier or touched the disk since the restore, so
    /// neither substitution can be observed as a change.
    ///
    /// # Errors
    ///
    /// Fails when the head does not have the shape the device was built for, or when the
    /// identifier is not one a guest may hold.
    pub(in crate::x86_64) fn assign_instance_resources(
        &self,
        overlay: Option<std::fs::File>,
        guest_cid: u32,
        network: Option<NetworkAttachment>,
    ) -> Result<(), MachineError> {
        // Validate every fallible identity input before replacing either detached resource.
        // Once this succeeds, `set_guest_cid` cannot reject the same value while the bus lock is
        // held, so assignment cannot expose a half-updated device set.
        VsockDevice::validate_guest_cid(u64::from(guest_cid))
            .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Vsock(error)))?;
        let backend = overlay
            .map(|overlay| FileBackend::new(overlay, false))
            .transpose()
            .map_err(|error| MachineError::io(Phase::Devices, &error))?;
        let mut bus = self.shared.lock();
        // A head for a machine with no overlay slot, or a slot with no head to fill it, is a
        // caller that thinks this is a different machine than it is; neither is resolved by
        // going ahead with whichever half was supplied.
        match (bus.overlay_mut(), backend) {
            (Some(overlay), Some(backend)) => overlay
                .device_mut()
                .attach(Box::new(backend))
                .map_err(|error| {
                    MachineError::new(Phase::Devices, MachineErrorKind::Block(error))
                })?,
            (None, None) => {}
            _ => {
                return Err(MachineError::invalid(
                    Phase::Devices,
                    "the machine's overlay slot and the assigned head disagree",
                ));
            }
        }
        bus.vsock_mut()
            .device_mut()
            .set_guest_cid(u64::from(guest_cid))
            .map_err(|error| MachineError::new(Phase::Devices, MachineErrorKind::Vsock(error)))?;
        // Attaching the frame path cannot fail, so it runs after every fallible step and keeps
        // the invariant above: no partly assigned device set is ever observable.
        //
        // An Instance without an admitted bundle keeps the device it was built with, which drops
        // every frame while the link is down. That is a machine with no network rather than one
        // whose network failed, and the two must not look alike.
        drop(bus);
        if let Some(network) = network {
            self.attach_network(network);
        }
        Ok(())
    }

    /// Wraps restored resources in the same machine the cold-boot path produces.
    ///
    /// # Errors
    ///
    /// Fails only when the host-work eventfd cannot be created.
    pub(in crate::x86_64) fn from_restored(parts: RestoredParts) -> Result<Self, MachineError> {
        let host_work = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Restore, &error))?;
        Ok(Self {
            machine: parts.machine,
            shared: Arc::new(SharedBus::new(parts.bus)),
            host_work: Arc::new(host_work),
            finished: Arc::new(AtomicBool::new(false)),
            launch_page: Mutex::new(Some(parts.launch_page)),
            console: None,
            exits: Arc::new(crate::x86_64::exits::ExitLedger::new()),
            stage: Stage::Prepared(Prepared {
                vcpu: parts.vcpu,
                serial_line: parts.serial_line,
                irq: parts.irq,
                notify: parts.notify,
            }),
            clock: parts.clock,
            timeline: Mutex::new(parts.timeline),
            cmdline: parts.cmdline,
            entry: 0,
            initramfs: None,
        })
    }
}
