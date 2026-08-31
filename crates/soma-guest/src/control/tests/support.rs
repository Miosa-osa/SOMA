use core::convert::Infallible;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use crate::{
    AuthenticatedSession, ControlIo, DeliveredHostLaunchMaterial, GuestLaunchMaterial,
    GuestMessage, GuestSessionMaterial, HostControlIo, HostLaunchMaterial, HostMessage,
    LAUNCH_PAGE_SIZE, LaunchNetwork,
};

pub(super) mod fault;

enum Packet {
    Bytes(Vec<u8>),
    Poisoned,
}

#[derive(Clone, Default)]
pub(super) struct Observation {
    poison: Arc<AtomicUsize>,
    repair: Arc<AtomicUsize>,
}

pub(super) struct MemoryIo {
    incoming: mpsc::Receiver<Packet>,
    outgoing: mpsc::Sender<Packet>,
    buffered: VecDeque<u8>,
    observed: Observation,
}

pub(super) struct RawGuest {
    io: MemoryIo,
    session: AuthenticatedSession,
}

pub(super) struct RawHost {
    io: MemoryIo,
    session: AuthenticatedSession,
}

pub(super) fn launch_network() -> LaunchNetwork {
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

pub(super) fn launch() -> (DeliveredHostLaunchMaterial, GuestSessionMaterial) {
    let host = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], launch_network())
        .expect("launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest page")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("entropy repair");
    (host, guest)
}

pub(super) fn pair() -> (MemoryIo, MemoryIo, Observation, Observation) {
    let (host_sender, guest_incoming) = mpsc::channel();
    let (guest_sender, host_incoming) = mpsc::channel();
    let host_observation = Observation::default();
    let guest_observation = Observation::default();
    (
        MemoryIo::new(host_incoming, host_sender, host_observation.clone()),
        MemoryIo::new(guest_incoming, guest_sender, guest_observation.clone()),
        host_observation,
        guest_observation,
    )
}

pub(super) fn deadline() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(30))
        .expect("test deadline is representable")
}

impl Observation {
    pub(super) fn poison(&self) -> usize {
        self.poison.load(Ordering::SeqCst)
    }

    pub(super) fn repair(&self) -> usize {
        self.repair.load(Ordering::SeqCst)
    }
}

impl MemoryIo {
    fn new(
        incoming: mpsc::Receiver<Packet>,
        outgoing: mpsc::Sender<Packet>,
        observed: Observation,
    ) -> Self {
        Self {
            incoming,
            outgoing,
            buffered: VecDeque::new(),
            observed,
        }
    }

    /// Whether the peer has sent nothing this side has not already consumed.
    ///
    /// The check is deliberately non-blocking: a test that wants to prove no further request was
    /// made cannot wait for one that is never coming.
    fn quiet(&mut self) -> bool {
        while let Ok(packet) = self.incoming.try_recv() {
            match packet {
                Packet::Bytes(bytes) => self.buffered.extend(bytes),
                Packet::Poisoned => return false,
            }
        }
        self.buffered.is_empty()
    }

    fn read_frame(&mut self, deadline: Instant) -> Result<Vec<u8>, ()> {
        let mut header = [0_u8; 2];
        self.read_exact(&mut header, deadline)?;
        let length = usize::from(u16::from_be_bytes(header));
        let mut frame = vec![0_u8; length + 2];
        frame[..2].copy_from_slice(&header);
        self.read_exact(&mut frame[2..], deadline)?;
        Ok(frame)
    }
}

impl ControlIo for MemoryIo {
    type Error = ();

    fn read_exact(&mut self, destination: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
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
        self.outgoing
            .send(Packet::Bytes(bytes.to_vec()))
            .map_err(|_| ())
    }

    fn poison(&mut self) {
        self.observed.poison.fetch_add(1, Ordering::SeqCst);
        let _ = self.outgoing.send(Packet::Poisoned);
    }
}

impl HostControlIo for MemoryIo {
    fn commit_repair(&mut self, deadline: Instant) -> Result<(), Self::Error> {
        if Instant::now() > deadline {
            return Err(());
        }
        self.observed.repair.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl RawGuest {
    pub(super) fn connect(material: GuestSessionMaterial, mut io: MemoryIo) -> Self {
        let deadline = deadline();
        let first = io.read_frame(deadline).expect("message one");
        let pending = material
            .start_responder(&first)
            .expect("responder handshake");
        io.write_all(pending.response(), deadline)
            .expect("message two");
        Self {
            io,
            session: pending.finish().expect("guest transport"),
        }
    }

    /// Whether the host has sent nothing further.
    pub(super) fn quiet(&mut self) -> bool {
        self.io.quiet()
    }

    pub(super) fn receive(&mut self) -> HostMessage {
        let record = self.io.read_frame(deadline()).expect("host record");
        let payload = self.session.open(&record).expect("host payload");
        HostMessage::decode(&payload).expect("host message")
    }

    pub(super) fn send(&mut self, message: GuestMessage) {
        let payload = message.encode().expect("guest message");
        drop(message);
        self.send_payload(&payload);
    }

    pub(super) fn send_payload(&mut self, payload: &[u8]) {
        let record = self.session.seal(payload).expect("guest record");
        self.io
            .write_all(&record, deadline())
            .expect("guest record write");
    }
}

impl RawHost {
    pub(super) fn connect(material: DeliveredHostLaunchMaterial, mut io: MemoryIo) -> Self {
        let (waiting, first) = material.start_initiator().expect("initiator handshake");
        let deadline = deadline();
        io.write_all(&first, deadline).expect("message one");
        let second = io.read_frame(deadline).expect("message two");
        Self {
            io,
            session: waiting.finish(&second).expect("host transport"),
        }
    }

    pub(super) fn send_payload(&mut self, payload: &[u8]) {
        let record = self.session.seal(payload).expect("host record");
        self.io
            .write_all(&record, deadline())
            .expect("host record write");
    }

    pub(super) fn receive(&mut self) -> GuestMessage {
        let record = self.io.read_frame(deadline()).expect("guest record");
        let payload = self.session.open(&record).expect("guest payload");
        GuestMessage::decode(&payload).expect("guest message")
    }
}
