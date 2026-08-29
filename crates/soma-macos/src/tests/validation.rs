use crate::{
    ExecutionLimits, GuestCommand, ImageReference, InstanceId, MachineShape, RequestErrorReason,
};

#[test]
fn request_newtypes_reject_ambiguous_or_unbounded_values() {
    assert!(ImageReference::new("x".repeat(1_024)).is_ok());
    assert_eq!(
        InstanceId::new("ABC".repeat(11))
            .expect_err("invalid identifier")
            .reason(),
        RequestErrorReason::InvalidIdentifier
    );
    assert_eq!(
        ImageReference::new("node:22 latest")
            .expect_err("spaces are forbidden")
            .reason(),
        RequestErrorReason::InvalidCharacter
    );
    assert_eq!(
        ImageReference::new("x".repeat(1_025))
            .expect_err("image references are bounded")
            .reason(),
        RequestErrorReason::TooLarge
    );
    for option_like_or_non_oci in ["-node:22", "https://example.invalid/node:22", r"node\22"] {
        assert_eq!(
            ImageReference::new(option_like_or_non_oci)
                .expect_err("unsupported image syntax")
                .reason(),
            RequestErrorReason::InvalidCharacter
        );
    }
    assert_eq!(
        GuestCommand::new("usr/bin/true", std::iter::empty::<String>())
            .expect_err("guest path must be absolute")
            .reason(),
        RequestErrorReason::NotAbsolute
    );
    assert_eq!(
        MachineShape::new(1, 1_000_000)
            .expect_err("memory is MiB aligned")
            .reason(),
        RequestErrorReason::NotMebibyteAligned
    );
    assert_eq!(
        ExecutionLimits::new(0, 1)
            .expect_err("timeout is bounded")
            .reason(),
        RequestErrorReason::Zero
    );
}
