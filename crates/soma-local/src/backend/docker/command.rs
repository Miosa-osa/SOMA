use std::time::Duration;

use super::process;

pub(super) const COMMAND: &str = "docker";
pub(super) const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
const CONTROL_OUTPUT_LIMIT: usize = 1_048_576;

pub(super) fn command(args: &[&str], timeout: Duration) -> process::Result {
    command_owned(
        &args
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        timeout,
    )
}

pub(super) fn command_owned(args: &[String], timeout: Duration) -> process::Result {
    process::run(COMMAND, args, timeout, CONTROL_OUTPUT_LIMIT).unwrap_or(process::Result {
        status: None,
        stdout: Vec::new(),
        stderr: Vec::new(),
        timed_out: false,
        output_limited: false,
    })
}
