//! Live proof that the bounded filesystem operations move real bytes into and out of a guest.
//!
//! Three things are proved here that no component test can prove. A file the host wrote is read
//! by a process inside the guest, so the write reached the guest's own filesystem rather than a
//! test double of it. A file that process wrote comes back to the host byte for byte. And a
//! payload larger than one record survives the chunking helpers, which is the only path a whole
//! file has across a protocol whose records are bounded.

use soma_guest::{FileOutcome, FileRequest, MAX_CHUNK_BYTES, WholeFileRead, WholeFileWrite};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_snapshot_restore_capability::{WORKSPACE, assert_no_leak, shell, succeeded},
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{self as workload, Session, Workload},
};

/// The file the host writes and a guest command reads.
const FROM_HOST: &[u8] = b"/tmp/soma-live/from-host.txt";
/// The file a guest command writes and the host reads.
const FROM_GUEST: &[u8] = b"/tmp/soma-live/from-guest.txt";
/// What the host puts in the first file, chosen so a shell can print it back unambiguously.
const HOST_TEXT: &[u8] = b"written by the host over the authenticated session\n";
/// The script that reads the host's file and writes one of its own beside it.
const EXCHANGE: &[u8] = b"cat /tmp/soma-live/from-host.txt \
     && printf 'written by a guest command at %s\\n' \"$(id -u)\" > /tmp/soma-live/from-guest.txt";
/// Ceiling on anything these proofs will hold in host memory.
const HOLD: usize = 1024 * 1024;

/// What the exchange proof retains.
pub struct Exchange {
    pub guest_saw: String,
    pub host_read: WholeFileRead,
}

struct ExchangeWorkload;

impl Workload for ExchangeWorkload {
    type Output = Exchange;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Exchange), String> {
        let (session, made) = session
            .file(FileRequest::MakeDirectory {
                path: WORKSPACE.into(),
                parents: true,
            })
            .map_err(|error| format!("make the workspace: {error}"))?;
        assert_eq!(made, FileOutcome::Done, "the workspace was not made");
        let (session, written) = session
            .write_whole_file(FROM_HOST, HOST_TEXT, HOLD)
            .map_err(|error| format!("write the host file: {error}"))?;
        assert_eq!(written, WholeFileWrite::Written);
        let (session, executed) = workload::execute(session, &shell(&[b"-c", EXCHANGE]))?;
        let guest_saw = succeeded("exchange", &executed);
        let (session, host_read) = session
            .read_whole_file(FROM_GUEST, HOLD)
            .map_err(|error| format!("read the guest file: {error}"))?;
        Ok((
            session,
            Exchange {
                guest_saw,
                host_read,
            },
        ))
    }
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_host_write_is_read_in_the_guest_and_a_guest_write_is_read_by_the_host() {
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "files-exchange", 40, ExchangeWorkload);
    assert_no_leak(&restored);

    let expected = String::from_utf8_lossy(HOST_TEXT).into_owned();
    assert_eq!(
        restored.output.guest_saw, expected,
        "a command in the guest did not read the bytes the host wrote"
    );
    let WholeFileRead::Bytes(bytes) = &restored.output.host_read else {
        panic!(
            "the host could not read back the guest's file: {:?}",
            restored.output.host_read
        );
    };
    let text = String::from_utf8_lossy(bytes);
    eprintln!("[files] the host read back {} bytes: {text:?}", bytes.len());
    assert!(
        text.starts_with("written by a guest command at 0\n"),
        "the guest's own bytes did not come back: {text:?}"
    );
}

/// What the chunked round trip retains.
pub struct RoundTrip {
    pub read: WholeFileRead,
    pub guest_length: String,
}

struct RoundTripWorkload {
    payload: Vec<u8>,
}

impl Workload for RoundTripWorkload {
    type Output = RoundTrip;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, RoundTrip), String> {
        let (session, made) = session
            .file(FileRequest::MakeDirectory {
                path: WORKSPACE.into(),
                parents: true,
            })
            .map_err(|error| format!("make the workspace: {error}"))?;
        assert_eq!(made, FileOutcome::Done);
        let (session, written) = session
            .write_whole_file(LARGE, &self.payload, HOLD)
            .map_err(|error| format!("write the large file: {error}"))?;
        assert_eq!(written, WholeFileWrite::Written);
        let (session, executed) = workload::execute(session, &shell(&[b"-c", MEASURE]))?;
        let guest_length = succeeded("measure", &executed);
        let (session, read) = session
            .read_whole_file(LARGE, HOLD)
            .map_err(|error| format!("read the large file: {error}"))?;
        Ok((session, RoundTrip { read, guest_length }))
    }
}

/// The file the chunked round trip moves.
const LARGE: &[u8] = b"/tmp/soma-live/large.bin";
/// The script that reports what the guest's own kernel says the file's length is.
const MEASURE: &[u8] = b"wc -c < /tmp/soma-live/large.bin";

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_file_larger_than_one_record_round_trips_byte_for_byte() {
    require_kvm();
    let fixture = fixture::shared();
    // Two full records and a partial one, so the loop is proved to carry a remainder as well as
    // to iterate at all, and every byte is position dependent so a misordered chunk is visible.
    let length = MAX_CHUNK_BYTES * 2 + 1237;
    let payload: Vec<u8> = (0..length).map(byte_at).collect();
    let restored = instance::run_workload(
        &fixture,
        "files-round-trip",
        41,
        RoundTripWorkload {
            payload: payload.clone(),
        },
    );
    assert_no_leak(&restored);

    eprintln!(
        "[files] one record carries at most {MAX_CHUNK_BYTES} bytes; the payload was {length}"
    );
    assert_eq!(
        restored.output.guest_length.trim(),
        length.to_string(),
        "the guest's own kernel reported a different length"
    );
    let WholeFileRead::Bytes(bytes) = &restored.output.read else {
        panic!("the read did not return bytes: {:?}", restored.output.read);
    };
    assert_eq!(bytes.len(), length, "the round trip changed the length");
    assert!(
        bytes == &payload,
        "the round trip changed a byte; first difference at {:?}",
        payload
            .iter()
            .zip(bytes)
            .position(|(sent, back)| sent != back)
    );
}

/// One position-dependent byte, so a chunk written at the wrong offset cannot compare equal.
fn byte_at(index: usize) -> u8 {
    u8::try_from(index % 251)
        .unwrap_or(0)
        .wrapping_mul(7)
        .wrapping_add(11)
}
