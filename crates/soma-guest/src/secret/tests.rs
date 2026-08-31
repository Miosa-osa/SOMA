use super::{DEFAULT_SECRET_FILE_MODE, InstancePsk, MAX_SECRET_BYTES, SecretFile, SecretValue};
use crate::Error;

/// The value every test here looks for in output it must never reach.
const VALUE: &[u8] = b"sk-live-6e2f9c41d0b7";

fn value() -> SecretValue {
    SecretValue::new(VALUE.to_vec()).expect("a bounded value")
}

#[test]
fn a_secret_value_reports_its_length_and_never_its_bytes() {
    let rendered = format!("{:?}", value());

    assert_eq!(rendered, "SecretValue { 20 bytes }");
}

#[test]
fn an_empty_or_oversized_secret_value_is_refused() {
    assert_eq!(
        SecretValue::new(Vec::new()).expect_err("an empty value"),
        Error::InvalidSecret
    );
    assert_eq!(
        SecretValue::new(vec![1; MAX_SECRET_BYTES + 1]).expect_err("an oversized value"),
        Error::InvalidSecret
    );
}

#[test]
fn a_secret_file_without_a_mode_is_owner_read_only() {
    let file = SecretFile::new(b"/run/secrets/token".to_vec(), None, value())
        .expect("an absolute destination");

    assert_eq!(file.mode(), DEFAULT_SECRET_FILE_MODE);
    assert_eq!(file.mode(), 0o400);
}

#[test]
fn a_secret_file_refuses_a_destination_or_mode_it_cannot_keep_private() {
    for (path, mode) in [
        (b"relative/token".to_vec(), None),
        (b"/run/secrets/token".to_vec(), Some(0o440)),
        (b"/run/secrets/token".to_vec(), Some(0o404)),
        (b"/run/secrets/token".to_vec(), Some(0o200)),
        (b"/run/secrets/token".to_vec(), Some(0o4400)),
    ] {
        assert_eq!(
            SecretFile::new(path, mode, value()).expect_err("an inadmissible secret file"),
            Error::InvalidSecret
        );
    }
}

#[test]
fn a_secret_file_reports_its_shape_and_never_its_value_or_destination() {
    let file = SecretFile::new(b"/run/secrets/token".to_vec(), Some(0o600), value())
        .expect("an absolute destination");
    let rendered = format!("{file:?}");

    assert_eq!(rendered, "SecretFile { path: 18 bytes, mode: 0o600 }");
    assert!(!rendered.contains("token"), "{rendered}");
    assert!(!rendered.contains("sk-live"), "{rendered}");
}

#[test]
fn a_destination_directly_under_the_root_needs_no_directory_made() {
    let file = SecretFile::new(b"/token".to_vec(), None, value()).expect("an absolute destination");

    assert_eq!(file.parent(), None);
}

#[test]
fn instance_psk_debug_output_never_exposes_secret_bytes() {
    let psk = InstancePsk::provision_for([2; 16], [0xAB; 32]).expect("instance PSK");

    assert_eq!(format!("{psk:?}"), "InstancePsk([REDACTED])");
    assert!(!format!("{psk:?}").contains("171"));
}

#[test]
fn zero_instance_psk_material_is_rejected() {
    assert_eq!(
        InstancePsk::provision_for([2; 16], [0; 32]).expect_err("zero PSK"),
        Error::InvalidKeyMaterial
    );
    assert_eq!(
        InstancePsk::provision_for([0; 16], [5; 32]).expect_err("zero Instance"),
        Error::InvalidKeyMaterial
    );
}
