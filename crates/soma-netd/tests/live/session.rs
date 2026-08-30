//! One real repaired guest-control session, used to mint activation receipts.
//!
//! The live tests must present the same capability a production Instance presents, so this
//! scaffolding runs the actual `soma-guest` handshake, authenticated repair, and fixed
//! readiness probe over an in-process byte transport before it mints anything.

use std::{
    collections::VecDeque,
    sync::mpsc,
    time::{Duration, Instant},
};

use soma_guest::{
    ActivationReceipt, ControlIo, GuestControl, GuestLaunchMaterial, HostControl, HostControlIo,
    HostLaunchMaterial, LAUNCH_PAGE_SIZE, LaunchNetwork, RepairedHostControl, TerminalStatus,
};
use soma_netd::Assigned;

/// The host side of one in-process authenticated control transport.
pub struct MemoryIo {
    incoming: mpsc::Receiver<Vec<u8>>,
    outgoing: mpsc::Sender<Vec<u8>>,
    buffered: VecDeque<u8>,
}

impl ControlIo for MemoryIo {
    type Error = ();

    fn read_exact(&mut self, destination: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
        while self.buffered.len() < destination.len() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            self.buffered
                .extend(self.incoming.recv_timeout(remaining).map_err(|_| ())?);
        }
        for byte in destination {
            *byte = self.buffered.pop_front().ok_or(())?;
        }
        Ok(())
    }

    fn write_all(&mut self, bytes: &[u8], _deadline: Instant) -> Result<(), Self::Error> {
        self.outgoing.send(bytes.to_vec()).map_err(|_| ())
    }

    fn poison(&mut self) {}
}

impl HostControlIo for MemoryIo {
    fn commit_repair(&mut self, _deadline: Instant) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn pair() -> (MemoryIo, MemoryIo) {
    let (host_sender, guest_incoming) = mpsc::channel();
    let (guest_sender, host_incoming) = mpsc::channel();
    (
        MemoryIo {
            incoming: host_incoming,
            outgoing: host_sender,
            buffered: VecDeque::new(),
        },
        MemoryIo {
            incoming: guest_incoming,
            outgoing: guest_sender,
            buffered: VecDeque::new(),
        },
    )
}

fn deadline() -> Instant {
    Instant::now()
        .checked_add(Duration::from_secs(30))
        .expect("representable deadline")
}

fn fixture_network() -> LaunchNetwork {
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
    .expect("fixture network")
}

/// Runs one authenticated session through repair and the fixed probe for these identities.
#[must_use]
pub fn repaired(instance: [u8; 16], operation: [u8; 16]) -> RepairedHostControl<MemoryIo> {
    let host = HostLaunchMaterial::generate([0x2c; 32], instance, operation, fixture_network())
        .expect("host launch material");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = host
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), ()>(())
        })
        .expect("page delivery");
    let guest = GuestLaunchMaterial::take_from_page(&mut page)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), ()>(()))
        .expect("guest entropy repair");
    let (host_io, guest_io) = pair();
    let guest_thread = std::thread::spawn(move || {
        let guest =
            GuestControl::connect(guest, guest_io, deadline()).expect("guest owner connected");
        let (guest, _) = guest.next_request(deadline()).expect("prepare request");
        guest
            .repair_complete(deadline())
            .expect("repair complete")
            .terminal(TerminalStatus::Exited(0), deadline())
            .expect("probe terminal")
    });
    let repaired = HostControl::connect(host, host_io)
        .expect("host owner connected")
        .prepare_and_probe()
        .expect("authenticated repair and probe");
    drop(guest_thread.join().expect("guest thread"));
    repaired
}

/// Mints the receipt this exact assignment requires from an already repaired session.
#[must_use]
pub fn mint(host: &RepairedHostControl<MemoryIo>, assigned: &Assigned) -> ActivationReceipt {
    host.network_activation(
        assigned
            .activation_challenge()
            .expect("unspent activation challenge"),
        assigned.record().generation.get(),
        assigned.record().intent_digest.0,
    )
    .expect("activation receipt")
}

/// Which part of the assignment a forged receipt disagrees with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wrong {
    /// A receipt minted by a different Instance's repaired session.
    Instance,
    /// A receipt bound to a different assignment generation.
    Generation,
    /// A receipt bound to a different admitted network intent.
    Intent,
}

/// Mints a real receipt from a real repaired session that disagrees on exactly one binding.
#[must_use]
pub fn forged(assigned: &Assigned, wrong: Wrong) -> ActivationReceipt {
    let record = assigned.record();
    let instance = if wrong == Wrong::Instance {
        [0x5a; 16]
    } else {
        *record.instance.as_bytes()
    };
    let generation = if wrong == Wrong::Generation {
        record.generation.get() + 1
    } else {
        record.generation.get()
    };
    let intent = if wrong == Wrong::Intent {
        [0x6b; 32]
    } else {
        record.intent_digest.0
    };
    repaired(instance, *record.operation.as_bytes())
        .network_activation(
            assigned
                .activation_challenge()
                .expect("unspent activation challenge"),
            generation,
            intent,
        )
        .expect("forged receipt")
}
