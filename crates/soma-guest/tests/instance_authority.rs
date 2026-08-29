//! P0.1 gates: every Instance authenticates with fresh authority that no public artifact
//! carries, and a party holding only public artifact bytes cannot impersonate the guest.

mod control_support;

use core::convert::Infallible;
use std::{
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use control_support::{MemoryIo, deadline, launch_network, pair};
use soma_guest::{
    ControlIo, DeliveredHostLaunchMaterial, GuestControl, GuestLaunchMaterial,
    GuestSessionMaterial, HostControl, HostControlIo, HostLaunchMaterial, LAUNCH_PAGE_SIZE,
};

/// The launch page bytes that carry this Instance's private authority.
const RESPONDER_SECRET: (usize, usize) = (247, 279);
const INSTANCE_PSK: (usize, usize) = (116, 148);

/// One Instance's crossed launch material plus the page bytes it travelled in.
struct Crossed {
    host: DeliveredHostLaunchMaterial,
    guest: GuestSessionMaterial,
    responder_public: [u8; 32],
    page: [u8; LAUNCH_PAGE_SIZE],
}

fn cross(generation: [u8; 32], instance: [u8; 16], operation: [u8; 16]) -> Crossed {
    let material = HostLaunchMaterial::generate(generation, instance, operation, launch_network())
        .expect("fresh Instance authority");
    let responder_public = material.responder_public_key().to_bytes();
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let host = material
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");
    assert_eq!(host.responder_public_key().to_bytes(), responder_public);
    let mut consumed = page;
    let guest = GuestLaunchMaterial::take_from_page(&mut consumed)
        .expect("guest launch material")
        .reseed_with(|_| Ok::<(), Infallible>(()))
        .expect("guest entropy repair");
    assert_eq!(consumed, [0; LAUNCH_PAGE_SIZE]);
    Crossed {
        host,
        guest,
        responder_public,
        page,
    }
}

/// Records every byte one peer writes so two sessions can be compared.
struct Tap {
    inner: MemoryIo,
    written: Arc<Mutex<Vec<u8>>>,
}

impl Tap {
    fn new(inner: MemoryIo) -> (Self, Arc<Mutex<Vec<u8>>>) {
        let written = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                inner,
                written: Arc::clone(&written),
            },
            written,
        )
    }
}

impl ControlIo for Tap {
    type Error = ();

    fn read_exact(&mut self, bytes: &mut [u8], deadline: Instant) -> Result<(), Self::Error> {
        self.inner.read_exact(bytes, deadline)
    }

    fn write_all(&mut self, bytes: &[u8], deadline: Instant) -> Result<(), Self::Error> {
        self.written
            .lock()
            .expect("transcript")
            .extend_from_slice(bytes);
        self.inner.write_all(bytes, deadline)
    }

    fn poison(&mut self) {
        self.inner.poison();
    }
}

impl HostControlIo for Tap {
    fn commit_repair(&mut self, deadline: Instant) -> Result<(), Self::Error> {
        self.inner.commit_repair(deadline)
    }
}

/// Completes one authenticated handshake and returns the host's exact written transcript.
fn handshake_transcript(crossed: Crossed) -> Vec<u8> {
    let (host_io, guest_io, _, _) = pair();
    let (host_io, written) = Tap::new(host_io);
    let guest = crossed.guest;
    let guest_thread =
        thread::spawn(move || GuestControl::connect(guest, guest_io, deadline()).is_ok());
    let host = HostControl::connect(crossed.host, host_io).expect("host owner connected");
    assert!(guest_thread.join().expect("guest thread"));
    drop(host);
    let transcript = written.lock().expect("transcript").clone();
    assert!(!transcript.is_empty());
    transcript
}

#[test]
fn two_instances_of_one_generation_authenticate_with_different_fresh_authority() {
    let generation = [0x5A; 32];
    let first = cross(generation, [1; 16], [2; 16]);
    let second = cross(generation, [3; 16], [4; 16]);

    assert_ne!(first.responder_public, second.responder_public);
    for (start, end) in [RESPONDER_SECRET, INSTANCE_PSK] {
        assert_ne!(first.page[start..end], second.page[start..end]);
    }
    let (start, end) = RESPONDER_SECRET;
    assert!(
        !first
            .page
            .windows(end - start)
            .any(|window| window == &second.page[start..end]),
        "one Instance's page must not contain another Instance's responder secret"
    );

    let first_transcript = handshake_transcript(first);
    let second_transcript = handshake_transcript(second);
    assert_ne!(first_transcript, second_transcript);
}

#[test]
fn a_party_holding_only_public_artifacts_cannot_impersonate_the_guest() {
    // Everything a party can retrieve: the Generation identity, the manifest bytes, the
    // machine images, and the published public responder identity of the live Instance.
    let generation = [0x7C; 32];
    let live = cross(generation, [8; 16], [9; 16]);
    let published: Vec<u8> = generation
        .iter()
        .copied()
        .chain(live.responder_public)
        .collect();
    for (start, end) in [RESPONDER_SECRET, INSTANCE_PSK] {
        assert!(
            !published
                .windows(end - start)
                .any(|window| window == &live.page[start..end]),
            "public artifacts must not contain private Instance authority"
        );
    }

    // The attacker derives whatever authority it can from those public bytes; without the
    // launch page it cannot obtain this Instance's responder secret or PSK.
    let mut instance = [0_u8; 16];
    instance.copy_from_slice(&published[..16]);
    let attacker = cross(generation, instance, [9; 16]);
    assert_ne!(attacker.responder_public, live.responder_public);

    let (host_io, guest_io, host_observed, _) = pair();
    let attacker_guest = attacker.guest;
    let guest_thread =
        thread::spawn(move || GuestControl::connect(attacker_guest, guest_io, deadline()).is_err());
    let result = HostControl::connect(live.host, host_io);

    assert!(
        result.is_err(),
        "an artifact-only party completed the handshake"
    );
    assert!(guest_thread.join().expect("attacker thread"));
    assert_eq!(host_observed.poison_calls(), 1);
}
