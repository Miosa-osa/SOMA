use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use soma_guest::{ControlIo, HostControlIo, LaunchNetwork};

#[derive(Debug)]
enum Packet {
    Bytes(Vec<u8>),
    Poisoned,
}

#[derive(Clone, Default)]
pub struct ObservedIo {
    poison_calls: Arc<Mutex<usize>>,
    repair_commits: Arc<Mutex<usize>>,
    read_deadlines: Arc<Mutex<Vec<Instant>>>,
    write_deadlines: Arc<Mutex<Vec<Instant>>>,
    repair_deadlines: Arc<Mutex<Vec<Instant>>>,
}

pub struct MemoryIo {
    incoming: mpsc::Receiver<Packet>,
    outgoing: mpsc::Sender<Packet>,
    buffered: VecDeque<u8>,
    observed: ObservedIo,
}

pub fn pair() -> (MemoryIo, MemoryIo, ObservedIo, ObservedIo) {
    let (host_sender, guest_incoming) = mpsc::channel();
    let (guest_sender, host_incoming) = mpsc::channel();
    let host_observed = ObservedIo::default();
    let guest_observed = ObservedIo::default();
    let host = MemoryIo {
        incoming: host_incoming,
        outgoing: host_sender,
        buffered: VecDeque::new(),
        observed: host_observed.clone(),
    };
    let guest = MemoryIo {
        incoming: guest_incoming,
        outgoing: guest_sender,
        buffered: VecDeque::new(),
        observed: guest_observed.clone(),
    };
    (host, guest, host_observed, guest_observed)
}

pub fn deadline() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(30))
        .expect("test deadline is representable")
}

impl ObservedIo {
    pub fn poison_calls(&self) -> usize {
        *self.poison_calls.lock().expect("poison counter")
    }

    pub fn repair_commits(&self) -> usize {
        *self.repair_commits.lock().expect("repair counter")
    }

    pub fn read_deadlines(&self) -> Vec<Instant> {
        self.read_deadlines.lock().expect("read deadlines").clone()
    }

    pub fn write_deadlines(&self) -> Vec<Instant> {
        self.write_deadlines
            .lock()
            .expect("write deadlines")
            .clone()
    }

    pub fn repair_deadlines(&self) -> Vec<Instant> {
        self.repair_deadlines
            .lock()
            .expect("repair deadlines")
            .clone()
    }
}

impl ControlIo for MemoryIo {
    type Error = ();

    fn read_exact(&mut self, destination: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
        self.observed
            .read_deadlines
            .lock()
            .expect("read deadlines")
            .push(deadline);
        while self.buffered.len() < destination.len() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.incoming.recv_timeout(remaining).map_err(|_| ())? {
                Packet::Bytes(bytes) => self.buffered.extend(bytes),
                Packet::Poisoned => return Err(()),
            }
        }
        for byte in destination {
            *byte = self.buffered.pop_front().ok_or(())?;
        }
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Self::Error> {
        if Instant::now() > deadline {
            return Err(());
        }
        self.observed
            .write_deadlines
            .lock()
            .expect("write deadlines")
            .push(deadline);
        self.outgoing
            .send(Packet::Bytes(bytes.to_vec()))
            .map_err(|_| ())
    }

    fn poison(&mut self) {
        *self.observed.poison_calls.lock().expect("poison counter") += 1;
        let _ = self.outgoing.send(Packet::Poisoned);
    }
}

impl HostControlIo for MemoryIo {
    fn commit_repair(&mut self, deadline: Instant) -> Result<(), Self::Error> {
        if Instant::now() > deadline {
            return Err(());
        }
        self.observed
            .repair_deadlines
            .lock()
            .expect("repair deadlines")
            .push(deadline);
        *self.observed.repair_commits.lock().expect("repair counter") += 1;
        Ok(())
    }
}

/// Returns the fixed non-secret network identity used by launch fixtures.
pub fn launch_network() -> LaunchNetwork {
    LaunchNetwork::new(
        3,
        1,
        [0x02, 0, 0, 0, 0, 1],
        [10, 0, 0, 2],
        24,
        [10, 0, 0, 1],
        [10, 0, 0, 1],
        1,
    )
    .expect("fixed test network")
}
