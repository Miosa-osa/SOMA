//! What placing a secret must put on the session, and what it must never put anywhere.

use std::thread;

use crate::{
    FileFailure, FileOutcome, FileRequest, GuestMessage, HostMessage, SecretFile, SecretPlacement,
    SecretStage, SecretValue,
};

use super::host_file::{operation, repaired_host};
use super::support::RawGuest;

/// The value every test here looks for in places it must never appear.
const VALUE: &[u8] = b"sk-live-6e2f9c41d0b7";
const DESTINATION: &[u8] = b"/run/soma/secrets/api-key";

fn secret() -> SecretFile {
    SecretFile::new(
        DESTINATION.to_vec(),
        None,
        SecretValue::new(VALUE.to_vec()).expect("a bounded value"),
    )
    .expect("an absolute destination and a default mode")
}

/// Answers one request with `outcome` and returns the request the host sent.
fn answer(raw: &mut RawGuest, outcome: FileOutcome) -> FileRequest {
    let HostMessage::File { operation, request } = raw.receive() else {
        panic!("the host sends a file request");
    };
    raw.send(GuestMessage::file_outcome(operation, outcome));
    request
}

#[test]
fn a_secret_is_created_privately_then_written_then_sealed() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.place_secret(&secret()));

    let directory = answer(&mut raw, FileOutcome::Done);
    let create = answer(&mut raw, FileOutcome::Done);
    let write = answer(&mut raw, FileOutcome::Written { bytes: 20 });
    let seal = answer(&mut raw, FileOutcome::Done);

    let (_host, placement) = host_thread.join().expect("host thread").expect("placement");
    assert_eq!(placement, SecretPlacement::Placed);
    assert_eq!(
        directory,
        FileRequest::MakeDirectory {
            path: b"/run/soma/secrets".as_slice().into(),
            parents: true,
        }
    );
    assert_eq!(
        create,
        FileRequest::Create {
            path: DESTINATION.into(),
            mode: 0o600,
        },
        "the destination exists at an owner-only mode before it holds anything"
    );
    assert_eq!(
        write,
        FileRequest::Write {
            path: DESTINATION.into(),
            offset: 0,
            create: true,
            shorten: true,
            bytes: VALUE.into(),
        }
    );
    assert_eq!(
        seal,
        FileRequest::SetMode {
            path: DESTINATION.into(),
            mode: 0o400,
        },
        "the Template's owner-read-only default is what the file ends at"
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn a_refused_creation_stops_before_the_value_reaches_the_wire() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.place_secret(&secret()));

    let _directory = answer(&mut raw, FileOutcome::Done);
    let _create = answer(&mut raw, FileOutcome::Failed(FileFailure::Exists));

    let (_host, placement) = host_thread.join().expect("host thread").expect("placement");
    assert_eq!(
        placement,
        SecretPlacement::Refused {
            stage: SecretStage::Create,
            failure: FileFailure::Exists,
        }
    );
    assert!(
        raw.quiet(),
        "a destination the guest would not create never receives the value"
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn a_refused_seal_is_reported_as_a_refusal_rather_than_a_placement() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.place_secret(&secret()));

    let _directory = answer(&mut raw, FileOutcome::Done);
    let _create = answer(&mut raw, FileOutcome::Done);
    let _write = answer(&mut raw, FileOutcome::Written { bytes: 20 });
    let _seal = answer(&mut raw, FileOutcome::Failed(FileFailure::Denied));

    let (_host, placement) = host_thread.join().expect("host thread").expect("placement");
    assert_eq!(
        placement,
        SecretPlacement::Refused {
            stage: SecretStage::Mode,
            failure: FileFailure::Denied,
        }
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn an_answer_of_the_wrong_shape_refuses_the_placement() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.place_secret(&secret()));

    let _directory = answer(&mut raw, FileOutcome::Status { kind: None });

    let (_host, placement) = host_thread.join().expect("host thread").expect("placement");
    assert_eq!(
        placement,
        SecretPlacement::Refused {
            stage: SecretStage::Directory,
            failure: FileFailure::Failed,
        }
    );
    assert_eq!(observed.poison(), 0);
}

#[test]
fn a_second_secret_is_never_offered_after_a_refused_one() {
    let (host, mut raw, observed) = repaired_host();
    let host_thread = thread::spawn(move || host.place_secrets(&[secret(), secret()]));

    let _directory = answer(&mut raw, FileOutcome::Failed(FileFailure::Denied));

    let (_host, placement) = host_thread.join().expect("host thread").expect("placement");
    assert_eq!(
        placement,
        SecretPlacement::Refused {
            stage: SecretStage::Directory,
            failure: FileFailure::Denied,
        }
    );
    assert!(raw.quiet(), "the second secret is not offered");
    assert_eq!(observed.poison(), 0);
}

#[test]
fn no_carrier_of_a_secret_renders_its_value() {
    let secret = secret();
    let value = SecretValue::new(VALUE.to_vec()).expect("a bounded value");
    let request = FileRequest::Write {
        path: DESTINATION.into(),
        offset: 0,
        create: true,
        shorten: true,
        bytes: VALUE.into(),
    };
    let message = HostMessage::file(operation(5), request.clone());
    let rendered = [
        format!("{value:?}"),
        format!("{secret:?}"),
        format!("{request:?}"),
        format!("{message:?}"),
        format!("{:?}", SecretPlacement::Placed),
        format!(
            "{:?}",
            SecretPlacement::Refused {
                stage: SecretStage::Write,
                failure: FileFailure::Denied,
            }
        ),
    ];
    let secret_text = String::from_utf8(VALUE.to_vec()).expect("an ASCII fixture");
    for text in &rendered {
        assert!(!text.contains(&secret_text), "{text} carries the value");
        assert!(
            !text.contains("sk-live"),
            "{text} carries part of the value"
        );
    }
}
