//! Live proof that a delivered secret arrives whole, at the mode that was asked for, and
//! nowhere else.
//!
//! Delivery is three bounded filesystem requests in a fixed order: the destination is created
//! exclusively at an owner-private mode, the value follows, and the final mode is applied last.
//! A component test can prove that order against a loopback. What it cannot prove is that a real
//! guest kernel ends with a file whose permission bits are exactly the ones requested and whose
//! bytes are exactly the value, and that nothing the run publishes carries the value.
//!
//! The negative half is the point of the whole capability, so it is asserted against everything
//! this run produces that anyone could read afterwards: the guest console, the session digest
//! the readiness receipt is minted over, and the snapshot objects every Instance of this
//! Generation shares.

use std::fs;
use std::path::Path;

use soma_guest::{SecretFile, SecretPlacement, SecretValue};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_sandbox_boot_session as session,
    x86_64_snapshot_restore_capability::{assert_no_leak, shell, succeeded},
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{self as workload, Session, Workload},
};

/// Where the secret is delivered, in a directory the delivery has to make for itself.
const DESTINATION: &[u8] = b"/tmp/soma-secret/token";
/// The mode the delivered file must end at: owner read, and nothing else at all.
const MODE: u32 = 0o400;
/// The script that reports the destination's mode, owner, size, and bytes.
const INSPECT: &[u8] =
    b"stat -c '%a %U %s' /tmp/soma-secret/token; stat -c '%a' /tmp/soma-secret; \
      cat /tmp/soma-secret/token";

/// What the delivery proof retains.
pub struct Delivered {
    pub placement: SecretPlacement,
    pub inspected: session::Executed,
    /// How the placement rendered itself, which nothing may recover the value from.
    pub rendered: String,
}

struct SecretWorkload {
    value: Vec<u8>,
}

impl Workload for SecretWorkload {
    type Output = Delivered;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Delivered), String> {
        let value =
            SecretValue::new(self.value.clone()).map_err(|error| format!("value: {error}"))?;
        let secret = SecretFile::new(DESTINATION.to_vec(), Some(MODE), value)
            .map_err(|error| format!("destination: {error}"))?;
        let (session, placement) = session
            .place_secret(&secret)
            .map_err(|error| format!("place the secret: {error}"))?;
        let rendered = format!("{placement:?}");
        let (session, inspected) = workload::execute(session, &shell(&[b"-c", INSPECT]))?;
        Ok((
            session,
            Delivered {
                placement,
                inspected,
                rendered,
            },
        ))
    }
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn a_delivered_secret_is_whole_at_the_requested_mode_and_in_no_evidence() {
    require_kvm();
    let fixture = fixture::shared();
    // The value is sampled fresh so that finding it anywhere cannot be a coincidence, and it is
    // printable so a shell in the guest can hand it back without an encoding in between.
    let value = printable_secret();
    let restored = instance::run_workload(
        &fixture,
        "secret",
        46,
        SecretWorkload {
            value: value.clone(),
        },
    );
    assert_no_leak(&restored);

    assert_eq!(
        restored.output.placement,
        SecretPlacement::Placed,
        "the delivery was refused"
    );
    let printed = succeeded("secret", &restored.output.inspected);
    let mut lines = printed.lines();
    let stat = lines.next().unwrap_or_default();
    let parent = lines.next().unwrap_or_default();
    let bytes: String = lines.collect();
    eprintln!("[secret] the guest reported: file={stat:?} parent={parent:?}");
    assert_eq!(
        stat,
        format!("400 root {}", value.len()),
        "the delivered file was not owner-read-only at the value's own length"
    );
    assert_eq!(
        bytes.as_bytes(),
        value.as_slice(),
        "the delivered bytes are not the value that was sent"
    );

    // Nothing the run publishes may carry the value. The console is what an operator reads, the
    // session digest is what the readiness receipt is minted over, and the snapshot objects are
    // what every other Instance of this Generation is restored from.
    assert!(
        !contains(&restored.evidence.serial, &value),
        "the value reached the guest console"
    );
    assert!(
        !contains(&restored.session_transcript, &value),
        "the value reached the session digest the receipt binds"
    );
    assert!(
        !restored
            .output
            .rendered
            .contains(std::str::from_utf8(&value).unwrap_or("")),
        "the placement rendered the value: {}",
        restored.output.rendered
    );
    for object in [
        fixture.paths.memory(),
        fixture.paths.overlay(),
        fixture.paths.state(),
    ] {
        assert!(
            !file_contains(&object, &value),
            "the value reached the shared snapshot object at {}",
            object.display()
        );
    }
    eprintln!(
        "[secret] {} value bytes appear in none of the console, the session digest, the \
         placement rendering, or the three shared snapshot objects",
        value.len()
    );
}

/// A fresh printable value, long enough that it cannot occur by chance in a scanned object.
fn printable_secret() -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    session::random16()
        .into_iter()
        .chain(session::random16())
        .chain(session::random16())
        .chain(session::random16())
        .map(|byte| ALPHABET[usize::from(byte) % ALPHABET.len()])
        .collect()
}

/// Whether a byte run holds the value anywhere in it.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Whether a published object holds the value anywhere in it.
///
/// The whole object is read rather than streamed, because a scan that skipped part of one would
/// not be evidence of anything, and the largest of them is the guest RAM this suite already
/// digests end to end elsewhere.
fn file_contains(path: &Path, needle: &[u8]) -> bool {
    let Ok(bytes) = fs::read(path) else {
        panic!(
            "the shared snapshot object at {} is unreadable",
            path.display()
        );
    };
    contains(&bytes, needle)
}
