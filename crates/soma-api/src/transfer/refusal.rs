//! How checkpoint transfer and import stage refusals read on the wire.

use crate::envelope::ApiError;

/// How a checkpoint transfer refusal reads on the wire.
#[must_use]
pub fn refusal(failure: soma_local::CheckpointFailure) -> ApiError {
    use soma_local::CheckpointFailure;
    match failure {
        CheckpointFailure::NotFound => ApiError::new(
            404,
            "checkpoint_not_found",
            "no published checkpoint matches the receipt",
            false,
        ),
        CheckpointFailure::CaptureRefused => ApiError::new(
            409,
            "checkpoint_transfer_refused",
            "the owner, receipt or received artifacts do not match the checkpoint",
            false,
        ),
        CheckpointFailure::Unsupported => ApiError::new(
            409,
            "checkpoint_transfer_unsupported",
            "this host has no encrypted checkpoint root",
            false,
        ),
        CheckpointFailure::Storage | CheckpointFailure::Unconfirmed => ApiError::new(
            503,
            "checkpoint_storage_unavailable",
            "the host did not confirm the checkpoint transfer",
            true,
        ),
    }
}

/// How an import stage refusal reads on the wire.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[must_use]
pub fn import_refusal(refused: soma_local::ImportRefusal) -> ApiError {
    use soma_local::ImportRefusal;
    match refused {
        ImportRefusal::Checkpoint(failure) => refusal(failure),
        ImportRefusal::Unknown => ApiError::new(
            404,
            "import_not_found",
            "no import stage has this identity",
            false,
        ),
        ImportRefusal::Busy => ApiError::new(
            409,
            "import_busy",
            "another request holds this import",
            true,
        ),
        ImportRefusal::TooManyImports => ApiError::new(
            429,
            "import_capacity",
            "this host holds as many open imports as it accepts",
            true,
        ),
        ImportRefusal::NoSpace => ApiError::new(
            507,
            "insufficient_storage",
            "the artifact does not fit the host's checkpoint storage",
            true,
        ),
    }
}
