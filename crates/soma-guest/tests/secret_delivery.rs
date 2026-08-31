//! Where a secret may appear, and where it may not.
//!
//! The launch page is the one thing about an Instance that exists before the session does, and
//! it is written into a machine that has not authenticated anything yet. A secret must therefore
//! never be part of it, and the page's fixed schema is what makes that true rather than a
//! convention: there is no field to put one in.
//!
//! Everything else here is the other half of the rule. A value that cannot be rendered cannot be
//! logged, so every public type that carries one is asked to render itself and checked.

use core::convert::Infallible;

use soma_guest::{
    FileFailure, FileRequest, HostLaunchMaterial, HostMessage, LAUNCH_PAGE_SIZE, LaunchNetwork,
    OperationId, SecretFile, SecretPlacement, SecretStage, SecretValue,
};

/// The value every assertion here looks for.
const VALUE: &[u8] = b"sk-live-6e2f9c41d0b7";

fn value() -> SecretValue {
    SecretValue::new(VALUE.to_vec()).expect("a bounded value")
}

fn secret() -> SecretFile {
    SecretFile::new(b"/run/soma/secrets/api-key".to_vec(), None, value())
        .expect("an absolute destination")
}

fn launch_network() -> LaunchNetwork {
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

#[test]
fn a_launch_page_carries_no_secret_value() {
    let material = HostLaunchMaterial::generate([1; 32], [2; 16], [3; 16], launch_network())
        .expect("fresh Instance authority");
    let mut page = [0_u8; LAUNCH_PAGE_SIZE];
    let _delivered = material
        .deliver_with(|bytes| {
            page.copy_from_slice(bytes);
            Ok::<(), Infallible>(())
        })
        .expect("page delivery");

    // The secret exists on the host at the same time as the page, and the page is the whole of
    // what the guest is given before it authenticates anything.
    let held = secret();
    assert_eq!(held.mode(), 0o400);
    assert!(
        !page.windows(VALUE.len()).any(|window| window == VALUE),
        "the launch page carries a secret value"
    );
}

#[test]
fn every_public_carrier_of_a_secret_renders_without_it() {
    let request = FileRequest::Write {
        path: b"/run/soma/secrets/api-key".as_slice().into(),
        offset: 0,
        create: true,
        shorten: true,
        bytes: VALUE.into(),
    };
    let message = HostMessage::file(
        OperationId::new([9; 16]).expect("an operation identity"),
        request.clone(),
    );
    let rendered = [
        format!("{:?}", value()),
        format!("{:?}", secret()),
        format!("{request:?}"),
        format!("{message:?}"),
        format!("{:?}", SecretPlacement::Placed),
        format!(
            "{:?}",
            SecretPlacement::Refused {
                stage: SecretStage::Create,
                failure: FileFailure::Exists,
            }
        ),
    ];

    let text = String::from_utf8(VALUE.to_vec()).expect("an ASCII fixture");
    for line in &rendered {
        assert!(!line.contains(&text), "{line} carries the value");
        assert!(!line.contains("sk-live"), "{line} carries part of it");
        assert!(!line.contains("api-key"), "{line} carries the destination");
    }
}
