//! One command a freshly launched machine runs for itself before it serves anyone.
//!
//! A restored guest finds its memory where the capture left it, but every page it touches for
//! the first time still costs an extended-page-table fault that KVM resolves on that access.
//! Populating the host mapping beforehand does not help: the second table is filled only by
//! the guest's own access, which is why the host-side prefault measured nothing. So the only
//! thing that makes the first command's pages resident is the guest touching them.
//!
//! The launch has already answered by the time this runs, so it costs the create nothing. A
//! client's first command reaches this host over a control plane round trip, and the warm
//! command spends that gap paying the faults the client's command would otherwise pay. A
//! command that arrives first waits on this one, because the host answers one request at a
//! time, and is then served exactly as before.
//!
//! It is off unless the service names a command. Its outcome is discarded: it is not a request
//! anyone made, so it has no receipt, and its only effect a caller can observe is that the
//! pages are resident. The guest agent reaps every process after every command, so nothing it
//! started outlives it.

use soma::InstanceId;
use soma_guest::GuestCommand;

use crate::backend::kvm::KvmBackend;

/// Names the command, as an absolute program path and its arguments separated by spaces.
const WARM_COMMAND: &str = "SOMA_LAUNCH_WARM_COMMAND";
/// Long enough for a language runtime to print its version; short enough that a client queued
/// behind a command that hangs is held for no more than this.
const WARM_TIMEOUT_MS: u32 = 2_000;
/// The output is discarded, so only enough is admitted for the command to finish normally.
const WARM_OUTPUT_BYTES: u64 = 64 * 1024;

/// Runs the configured warm command once on the machine this host holds, if one is configured.
pub(super) fn warm(backend: &mut KvmBackend, instance: &InstanceId) {
    let Some(command) = std::env::var(WARM_COMMAND)
        .ok()
        .as_deref()
        .and_then(command)
    else {
        return;
    };
    let _ignored = backend.execute_resident(instance, command);
}

/// Parses the configured command, refusing anything the guest would refuse.
fn command(value: &str) -> Option<GuestCommand> {
    let mut words = value
        .split_ascii_whitespace()
        .map(|word| word.as_bytes().to_vec());
    let program = words.next()?;
    GuestCommand::new(program, words.collect(), WARM_TIMEOUT_MS, WARM_OUTPUT_BYTES).ok()
}

#[cfg(test)]
mod tests {
    use super::command;

    #[test]
    fn a_program_and_its_arguments_are_parsed() {
        assert!(command("/usr/local/bin/node -v").is_some());
        assert!(command("  /bin/true  ").is_some());
    }

    #[test]
    fn an_empty_or_relative_command_is_never_run() {
        assert!(command("").is_none());
        assert!(command("   ").is_none());
        assert!(command("node -v").is_none());
    }
}
