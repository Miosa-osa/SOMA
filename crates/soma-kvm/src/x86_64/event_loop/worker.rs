//! The device worker thread: drain host work, network frames and queue notifications in bounded passes.

use super::{
    Epoll, EpollEvent, EventFd, EventLoopReport, INTERRUPT_CONFIG_CHANGE, IrqLines, NET_RX_QUEUE,
    NotifyFds, PASS_LIMIT, ServiceError, ServiceReport, SharedBus, SharedRam, Slot,
    TOKEN_HOST_WORK, TOKEN_NET_RX, TOKEN_STOP, WORK_BUDGET, bump,
};
use std::{io, sync::Arc};

pub(super) struct Worker {
    pub(super) shared: Arc<SharedBus>,
    pub(super) memory: SharedRam,
    pub(super) notify: NotifyFds,
    pub(super) irq: IrqLines,
    pub(super) host_work: EventFd,
    pub(super) epoll: Epoll,
    pub(super) report: EventLoopReport,
}

impl Worker {
    pub(super) fn run(mut self) -> Self {
        let mut events = vec![EpollEvent::default(); self.notify.entries().len() + 3];
        'serve: loop {
            let ready = match self.epoll.wait(-1, &mut events) {
                Ok(ready) => ready,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break 'serve,
            };
            for event in &events[..ready] {
                match event.data() {
                    TOKEN_STOP => break 'serve,
                    TOKEN_HOST_WORK => self.host_work(),
                    TOKEN_NET_RX => {
                        bump(&mut self.report.host_wakeups);
                        self.net_frames();
                    }
                    token => self.queue_work(token),
                }
            }
        }
        self
    }

    /// Delivers whatever the host side already holds for the guest.
    ///
    /// Both host-fed slots are served, because the eventfd says only that the host produced
    /// work, not which device it belongs to: a caller that raises the network link has host
    /// frames to place exactly as a control-channel writer has vsock packets to place.
    pub(super) fn host_work(&mut self) {
        let _ignored = self.host_work.read();
        bump(&mut self.report.host_wakeups);
        let outcome = self
            .shared
            .lock()
            .deliver_inbound(Slot::Vsock, &self.memory, WORK_BUDGET);
        self.finish_pass(Slot::Vsock, 0, outcome);
        self.net_frames();
        self.memory_config_change();
    }

    /// Raises the memory device's interrupt when the host has changed its requested size and the
    /// guest has not yet acknowledged it.
    ///
    /// The interrupt line belongs to this thread, so a host that flags a configuration change
    /// wakes it rather than signalling the line itself.
    pub(super) fn memory_config_change(&mut self) {
        let pending = self.shared.lock().interrupt_status(Slot::Mem) & INTERRUPT_CONFIG_CHANGE != 0;
        if pending && self.irq.signal_slot(Slot::Mem) {
            bump(&mut self.report.slots[usize::from(Slot::Mem.index())].interrupts);
        }
    }

    /// Delivers frames the attached network backend already holds.
    ///
    /// The pass limit bounds one wakeup exactly as a queue notification is bounded, so a host
    /// that keeps the backend readable cannot starve the control channel.
    pub(super) fn net_frames(&mut self) {
        for _ in 0..PASS_LIMIT {
            let outcome = self
                .shared
                .lock()
                .deliver_inbound(Slot::Net, &self.memory, WORK_BUDGET);
            if !self.finish_pass(Slot::Net, NET_RX_QUEUE, outcome) {
                return;
            }
        }
    }

    pub(super) fn queue_work(&mut self, token: u64) {
        let Some(index) = usize::try_from(token).ok() else {
            return;
        };
        let Some(entry) = self.notify.entries().get(index) else {
            return;
        };
        let (slot, queue) = (entry.slot, entry.queue);
        let _ignored = entry.fd.read();
        bump(&mut self.report.slots[usize::from(slot.index())].wakeups);
        for _ in 0..PASS_LIMIT {
            let outcome = self
                .shared
                .lock()
                .service(slot, queue, &self.memory, WORK_BUDGET);
            if !self.finish_pass(slot, queue, outcome) {
                return;
            }
        }
        // Work remains after the pass limit: re-arm and yield to the other descriptors.
        if let Some(entry) = self.notify.entries().get(index) {
            let _ignored = entry.fd.write(1);
        }
    }

    /// Signals the slot's interrupt when the driver wants it and wakes control waiters;
    /// returns whether the queue still has work to do.
    pub(super) fn finish_pass(
        &mut self,
        slot: Slot,
        queue: u16,
        outcome: Result<ServiceReport, ServiceError>,
    ) -> bool {
        let activity = &mut self.report.slots[usize::from(slot.index())];
        let exhausted = if let Ok(report) = outcome {
            activity.completed = activity
                .completed
                .saturating_add(u64::from(report.completed));
            activity.rejected = activity.rejected.saturating_add(u64::from(report.rejected));
            if report.interrupt && self.irq.signal_slot(slot) {
                bump(&mut activity.interrupts);
            }
            report.exhausted
        } else {
            bump(&mut activity.faults);
            if self.report.first_fault.is_none() {
                self.report.first_fault = Some((slot, queue));
            }
            false
        };
        self.shared.notify_all();
        exhausted
    }
}
