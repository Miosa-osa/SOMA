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
use crate::virtio::{
    INTERRUPT_CONFIG_CHANGE, NET_RX_QUEUE, SLOT_COUNT, ServiceError, ServiceReport, Slot,
};
mod worker;
use worker::Worker;

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

/// The eventfds assigned to one VM's device bus.
pub(crate) struct DeviceRoutes {
    pub(crate) notify: NotifyFds,
    pub(crate) irq: IrqLines,
}

/// The running device thread and the descriptor that stops it.
pub(crate) struct EventLoop {
    worker: Option<JoinHandle<Worker>>,
    stop: Option<EventFd>,
}

/// A quiescent device loop retaining its queue registrations and epoll instance.
pub(crate) struct ParkedDevices {
    worker: Worker,
    stop: EventFd,
}

impl ParkedDevices {
    pub(crate) fn resume(self) -> io::Result<EventLoop> {
        // Consume the wakeup that ended the previous thread before re-entering epoll.
        self.stop.read()?;
        EventLoop::start_worker(self.worker, self.stop)
    }

    pub(crate) fn release(self) -> (EventLoopReport, NotifyFds, IrqLines) {
        (self.worker.report, self.worker.notify, self.worker.irq)
    }
}

impl EventLoop {
    /// Registers every descriptor with a fresh epoll instance and starts the thread.
    pub(crate) fn spawn(
        shared: Arc<SharedBus>,
        memory: SharedRam,
        routes: DeviceRoutes,
        host_work: EventFd,
        net_rx: Option<RawFd>,
    ) -> io::Result<Self> {
        let DeviceRoutes { notify, irq } = routes;
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
        Self::start_worker(worker, stop)
    }

    fn start_worker(worker: Worker, stop: EventFd) -> io::Result<Self> {
        let handle = thread::Builder::new()
            .name("soma-kvm-devices".to_owned())
            .spawn(move || worker.run())?;
        Ok(Self {
            worker: Some(handle),
            stop: Some(stop),
        })
    }

    /// Joins the thread while retaining the resources it can resume with.
    pub(crate) fn park(mut self) -> Option<ParkedDevices> {
        let stop = self.stop.take()?;
        let _ignored = stop.write(1);
        let worker = self.worker.take()?.join().ok()?;
        Some(ParkedDevices { worker, stop })
    }

    /// Stops the thread and returns its report together with the descriptors it owned.
    pub(crate) fn stop(self) -> Option<(EventLoopReport, NotifyFds, IrqLines)> {
        self.park().map(ParkedDevices::release)
    }
}

impl Drop for EventLoop {
    fn drop(&mut self) {
        if let (Some(worker), Some(stop)) = (self.worker.take(), self.stop.take()) {
            let _ignored = stop.write(1);
            let _ignored = worker.join();
        }
    }
}
