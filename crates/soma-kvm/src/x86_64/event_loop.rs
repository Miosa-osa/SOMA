//! The single device thread: epoll over every queue-notify ioeventfd, the host-work eventfd,
//! and the stop eventfd; bounded servicing of the shared bus; irqfd signalling.
//!
//! One wakeup services at most `WORK_BUDGET` chains per pass and at most `PASS_LIMIT` passes
//! before it re-arms its own eventfd and yields, so a guest that keeps a queue full cannot
//! monopolise the thread or starve the control channel.
//!
//! An attached network backend is watched here too. The guest notifies the receive queue only
//! when it posts buffers, so without this registration a frame that arrived from the host would
//! wait for a notification the guest has no reason to send, and every reply to guest traffic
//! would stall. The registration is edge triggered: while the link is down, or while the guest
//! has posted no receive buffer, delivery reads nothing from the backend, and a level-triggered
//! descriptor would then wake this thread continuously for a frame it cannot place. Edge
//! triggering costs a wakeup only when the host side transitions to readable, and the guest's
//! own receive-queue notification drains whatever a skipped pass left behind.

use std::{
    io,
    os::fd::{AsRawFd as _, RawFd},
    sync::Arc,
    thread::{self, JoinHandle},
};

use vmm_sys_util::{
    epoll::{ControlOperation, Epoll, EpollEvent, EventSet},
    eventfd::EventFd,
};

use super::{devices::SharedBus, events::IrqLines, events::NotifyFds, memory::SharedRam};
use crate::virtio::{NET_RX_QUEUE, SLOT_COUNT, ServiceError, ServiceReport, Slot};

const WORK_BUDGET: u32 = 64;
const PASS_LIMIT: u32 = 8;
const TOKEN_STOP: u64 = u64::MAX;
const TOKEN_HOST_WORK: u64 = u64::MAX - 1;
const TOKEN_NET_RX: u64 = u64::MAX - 2;

/// Bounded per-slot counts of what the device thread did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlotActivity {
    pub wakeups: u64,
    pub completed: u64,
    pub interrupts: u64,
    pub rejected: u64,
    pub faults: u64,
}

/// What the device thread did, returned when it is joined.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EventLoopReport {
    pub slots: [SlotActivity; SLOT_COUNT],
    pub host_wakeups: u64,
    pub first_fault: Option<(Slot, u16)>,
}

fn bump(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

struct Worker {
    shared: Arc<SharedBus>,
    memory: SharedRam,
    notify: NotifyFds,
    irq: IrqLines,
    host_work: EventFd,
    epoll: Epoll,
    report: EventLoopReport,
}

impl Worker {
    fn run(mut self) -> (EventLoopReport, NotifyFds, IrqLines) {
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
        (self.report, self.notify, self.irq)
    }

    /// Delivers whatever the host side already holds for the guest.
    ///
    /// Both host-fed slots are served, because the eventfd says only that the host produced
    /// work, not which device it belongs to: a caller that raises the network link has host
    /// frames to place exactly as a control-channel writer has vsock packets to place.
    fn host_work(&mut self) {
        let _ignored = self.host_work.read();
        bump(&mut self.report.host_wakeups);
        let outcome = self
            .shared
            .lock()
            .deliver_inbound(Slot::Vsock, &self.memory, WORK_BUDGET);
        self.finish_pass(Slot::Vsock, 0, outcome);
        self.net_frames();
    }

    /// Delivers frames the attached network backend already holds.
    ///
    /// The pass limit bounds one wakeup exactly as a queue notification is bounded, so a host
    /// that keeps the backend readable cannot starve the control channel.
    fn net_frames(&mut self) {
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

    fn queue_work(&mut self, token: u64) {
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
    fn finish_pass(
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

/// The running device thread and the descriptor that stops it.
pub(crate) struct EventLoop {
    worker: Option<JoinHandle<(EventLoopReport, NotifyFds, IrqLines)>>,
    stop: EventFd,
}

impl EventLoop {
    /// Registers every descriptor with a fresh epoll instance and starts the thread.
    pub(crate) fn spawn(
        shared: Arc<SharedBus>,
        memory: SharedRam,
        notify: NotifyFds,
        irq: IrqLines,
        host_work: EventFd,
        net_rx: Option<RawFd>,
    ) -> io::Result<Self> {
        let stop = EventFd::new(libc::EFD_NONBLOCK)?;
        let epoll = Epoll::new()?;
        for (index, entry) in notify.entries().iter().enumerate() {
            let token = u64::try_from(index).map_err(|_| io::Error::other("token overflow"))?;
            epoll.ctl(
                ControlOperation::Add,
                entry.fd.as_raw_fd(),
                EpollEvent::new(EventSet::IN, token),
            )?;
        }
        epoll.ctl(
            ControlOperation::Add,
            host_work.as_raw_fd(),
            EpollEvent::new(EventSet::IN, TOKEN_HOST_WORK),
        )?;
        if let Some(net_rx) = net_rx {
            epoll.ctl(
                ControlOperation::Add,
                net_rx,
                EpollEvent::new(EventSet::IN | EventSet::EDGE_TRIGGERED, TOKEN_NET_RX),
            )?;
        }
        epoll.ctl(
            ControlOperation::Add,
            stop.as_raw_fd(),
            EpollEvent::new(EventSet::IN, TOKEN_STOP),
        )?;
        let worker = Worker {
            shared,
            memory,
            notify,
            irq,
            host_work,
            epoll,
            report: EventLoopReport::default(),
        };
        let handle = thread::Builder::new()
            .name("soma-kvm-devices".to_owned())
            .spawn(move || worker.run())?;
        Ok(Self {
            worker: Some(handle),
            stop,
        })
    }

    /// Stops the thread and returns its report together with the descriptors it owned.
    pub(crate) fn stop(mut self) -> Option<(EventLoopReport, NotifyFds, IrqLines)> {
        let _ignored = self.stop.write(1);
        self.worker.take().and_then(|worker| worker.join().ok())
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ignored = self.stop.write(1);
            let _ignored = worker.join();
        }
    }
}
