//! The create size vocabulary: which machine `size` selects, and the network it carries.
//!
//! The two sizes are checked here against the shape they resolve to rather than against a
//! constant, so a runner that misconfigures its large shape is refused on this path.

use super::{
    params::CreateParams,
    tests::{launch, launch_serving_large, shape},
};
use crate::runner::config::{LargeShape, LaunchConfig};

#[test]
fn a_large_create_is_served_the_large_shape() {
    let parsed = CreateParams::parse(br#"{"size":"large"}"#, 3_600, &launch_serving_large())
        .expect("a runner that configured the large shape serves it");

    assert_eq!(parsed.shape.vcpu_count(), 8);
    assert_eq!(parsed.shape.memory_mib(), 16 * 1024);
    assert_eq!(parsed.shape.storage_mib(), 20 * 1024);
    assert_eq!(
        parsed.shape.capabilities().network_policy().egress(),
        soma::EgressPolicy::PublicInternet,
        "a large create reaches the public internet by default"
    );
    assert_eq!(
        parsed.shape.capabilities().network_policy().dns(),
        &soma::DnsPolicy::System,
    );
}

#[test]
fn a_large_create_is_refused_by_a_runner_that_does_not_serve_it() {
    // The refusal is the same one an unknown size gets, so a caller cannot tell a size this
    // host does not build from a size the platform does not define.
    let refused = CreateParams::parse(br#"{"size":"large"}"#, 3_600, &launch())
        .expect_err("a runner without a large shape refuses one");

    assert_eq!(refused.status, 400);
    assert_eq!(refused.code, "RUNNER_UNSUPPORTED_REQUEST");
}

#[test]
fn the_size_field_only_ever_selects_one_of_the_two_named_shapes() {
    let config = launch_serving_large();
    // `xs` and an absent size both select the runner's own shape, so the byte-for-byte
    // behaviour of every create written before `large` existed is unchanged.
    for body in [r#"{"size":"xs"}"#, "{}"] {
        let parsed = CreateParams::parse(body.as_bytes(), 3_600, &config).expect("accepted");
        assert_eq!(parsed.shape, shape(), "{body} selects the runner's shape");
    }
    for body in [
        r#"{"size":"m"}"#,
        r#"{"size":"medium"}"#,
        r#"{"size":"Large"}"#,
        r#"{"size":null}"#,
        r#"{"size":8}"#,
    ] {
        assert_eq!(
            CreateParams::parse(body.as_bytes(), 3_600, &config).map_err(|error| error.code),
            Err("RUNNER_UNSUPPORTED_REQUEST"),
            "{body} must be refused"
        );
    }
}

#[test]
fn a_large_create_is_checked_against_the_large_shape() {
    let config = launch_serving_large();
    assert!(
        CreateParams::parse(
            br#"{"size":"large","cpu_count":8,"memory_mb":16384}"#,
            3_600,
            &config
        )
        .is_ok(),
        "the large shape's own numbers are accepted"
    );
    for body in [
        r#"{"size":"large","cpu_count":1}"#,
        r#"{"size":"large","cpu_count":4}"#,
        r#"{"size":"large","memory_mb":512}"#,
        r#"{"size":"large","memory_mb":32768}"#,
    ] {
        assert!(
            CreateParams::parse(body.as_bytes(), 3_600, &config).is_err(),
            "{body} must be refused against the large shape"
        );
    }
}

#[test]
fn a_machine_of_the_large_size_can_be_built_without_a_network() {
    let config = LaunchConfig {
        large: Some(LargeShape {
            public_egress: false,
            ..launch_serving_large().large.expect("large")
        }),
        ..launch()
    };
    let parsed = CreateParams::parse(br#"{"size":"large"}"#, 3_600, &config).expect("accepted");
    assert_eq!(
        parsed.shape.capabilities().network_policy().egress(),
        soma::EgressPolicy::Denied,
        "public_egress: false builds the same machine with no network"
    );
}
