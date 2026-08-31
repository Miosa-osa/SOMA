//! Live proof that a refused filesystem request is refused for the reason it actually happened.
//!
//! The protocol reduces every kernel error to one of six causes, and a component test can only
//! prove that reduction against errors it manufactured. What it cannot prove is that a real
//! guest kernel, on a real image, produces the errno the reduction expects: an absent path, a
//! directory read as a file, a non-empty directory removed without consent, and a path created
//! twice each have to reach the caller as their own cause rather than as the catch-all.
//!
//! The session stays usable throughout. A refusal is an answer, not a transport failure, and a
//! proof that ends with a working shutdown is what says so.

use soma_guest::{FileFailure, FileOutcome, FileRequest};
use soma_kvm::x86_64::SandboxMachine;

use crate::{
    x86_64_sandbox_boot_host::require_kvm,
    x86_64_snapshot_restore_capability::{assert_no_leak, shell, succeeded},
    x86_64_snapshot_restore_fixture as fixture, x86_64_snapshot_restore_instance as instance,
    x86_64_snapshot_restore_workload::{self as workload, Session, Workload},
};

/// A path nothing in a `node:22` image occupies.
const ABSENT: &[u8] = b"/tmp/soma-refusal/absent";
/// A directory, so a request that requires a file has one of the wrong kind to name.
const DIRECTORY: &[u8] = b"/tmp/soma-refusal/holder";
/// The file inside it, which makes the directory non-empty.
const OCCUPANT: &[u8] = b"/tmp/soma-refusal/holder/occupant";
/// The script that builds both.
const BUILD: &[u8] = b"set -e; rm -rf /tmp/soma-refusal; mkdir -p /tmp/soma-refusal/holder; \
     : > /tmp/soma-refusal/holder/occupant; ls -1 /tmp/soma-refusal/holder";

/// Every refusal one restored Instance produced, in the order they were asked for.
pub struct Refusals {
    pub built: String,
    pub absent_read: FileOutcome,
    pub absent_status: FileOutcome,
    pub wrong_kind: FileOutcome,
    pub not_empty: FileOutcome,
    pub exists: FileOutcome,
    pub emptied: FileOutcome,
}

struct RefusalWorkload;

impl Workload for RefusalWorkload {
    type Output = Refusals;

    fn run<'a>(
        &mut self,
        _machine: &'a SandboxMachine,
        session: Session<'a>,
    ) -> Result<(Session<'a>, Refusals), String> {
        let (session, executed) = workload::execute(session, &shell(&[b"-c", BUILD]))?;
        let built = succeeded("build", &executed);
        let (session, absent_read) = ask(
            session,
            FileRequest::Read {
                path: ABSENT.into(),
                offset: 0,
                length: 16,
            },
        )?;
        // Absence is an answer to `Exists` rather than a refusal of it, and the two must not
        // look alike: a caller asking whether a path is there gets a status, and a caller
        // reading one that is not gets the cause.
        let (session, absent_status) = ask(
            session,
            FileRequest::Exists {
                path: ABSENT.into(),
            },
        )?;
        let (session, wrong_kind) = ask(
            session,
            FileRequest::Read {
                path: DIRECTORY.into(),
                offset: 0,
                length: 16,
            },
        )?;
        let (session, not_empty) = ask(
            session,
            FileRequest::Remove {
                path: DIRECTORY.into(),
                recursive: false,
            },
        )?;
        let (session, exists) = ask(
            session,
            FileRequest::Create {
                path: OCCUPANT.into(),
                mode: 0o600,
            },
        )?;
        // Removing the occupant and then the directory proves the refusal above was about the
        // directory being non-empty rather than about the caller being unable to remove it.
        let (session, _) = ask(
            session,
            FileRequest::Remove {
                path: OCCUPANT.into(),
                recursive: false,
            },
        )?;
        let (session, emptied) = ask(
            session,
            FileRequest::Remove {
                path: DIRECTORY.into(),
                recursive: false,
            },
        )?;
        Ok((
            session,
            Refusals {
                built,
                absent_read,
                absent_status,
                wrong_kind,
                not_empty,
                exists,
                emptied,
            },
        ))
    }
}

/// Issues one filesystem request and keeps its answer.
fn ask(session: Session<'_>, request: FileRequest) -> Result<(Session<'_>, FileOutcome), String> {
    session
        .file(request)
        .map_err(|error| format!("filesystem request: {error}"))
}

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and a node:22 OCI layout"]
fn every_filesystem_refusal_names_the_reason_it_actually_happened() {
    require_kvm();
    let fixture = fixture::shared();
    let restored = instance::run_workload(&fixture, "files-refusal", 43, RefusalWorkload);
    assert_no_leak(&restored);

    let refusals = &restored.output;
    assert_eq!(refusals.built.trim(), "occupant");
    eprintln!(
        "[refusal] absent={:?} status={:?} wrong_kind={:?} not_empty={:?} exists={:?} emptied={:?}",
        refusals.absent_read,
        refusals.absent_status,
        refusals.wrong_kind,
        refusals.not_empty,
        refusals.exists,
        refusals.emptied
    );
    assert_eq!(
        refusals.absent_read,
        FileOutcome::Failed(FileFailure::NotFound),
        "reading an absent path did not report absence"
    );
    assert_eq!(
        refusals.absent_status,
        FileOutcome::Status { kind: None },
        "asking about an absent path was answered as a failure"
    );
    assert_eq!(
        refusals.wrong_kind,
        FileOutcome::Failed(FileFailure::WrongKind),
        "reading a directory as a file did not report the wrong kind"
    );
    assert_eq!(
        refusals.not_empty,
        FileOutcome::Failed(FileFailure::NotEmpty),
        "removing a non-empty directory did not report that it was not empty"
    );
    assert_eq!(
        refusals.exists,
        FileOutcome::Failed(FileFailure::Exists),
        "creating an occupied path did not report that it was occupied"
    );
    assert_eq!(
        refusals.emptied,
        FileOutcome::Done,
        "the same directory could not be removed once it was empty"
    );
}
