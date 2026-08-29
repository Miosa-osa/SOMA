use std::{env, path::PathBuf};

use soma_macos::{ControlLimits, InstanceId};

const CONTROL_TIMEOUT_MS: u64 = 30_000;
const CONTROL_OUTPUT_BYTES: u64 = 1_048_576;

pub(super) const STOP_GRACE_SECONDS: u32 = 10;

pub(super) fn resolve_runtime(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(runtime) = explicit {
        return runtime;
    }
    if let Some(home) = env::var_os("HOME") {
        let pinned = PathBuf::from(home)
            .join("Library/Application Support/SOMA/apple-container/1.3.0/bin/container");
        if pinned.is_file() {
            return pinned;
        }
    }
    PathBuf::from("container")
}

pub(super) fn control_limits() -> ControlLimits {
    ControlLimits::new(CONTROL_TIMEOUT_MS, CONTROL_OUTPUT_BYTES)
        .expect("fixed control limits satisfy the Apple adapter contract")
}

pub(super) fn mac_instance(value: &str) -> Result<InstanceId, soma_macos::RequestError> {
    InstanceId::new(value)
}
