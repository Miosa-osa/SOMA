//! The agent's `--version` report.
//!
//! The agent records no commit, on purpose. It is built into every Generation and a Generation
//! is addressed by its content, so a commit string inside the agent would give every new commit
//! a new Generation identity even when the agent's code had not changed. The release manifest
//! ties the agent's digest to the commit of its release instead.

use soma_version::{Build, Protocol, Speaks};

/// The guest protocols this agent speaks, with the versions this build speaks.
const PROTOCOLS: [Protocol; 4] = [
    Protocol::new(
        "guest-application",
        soma_guest::APPLICATION_PROTOCOL_VERSION as u32,
    ),
    Protocol::new("guest-api", soma_guest::GUEST_API_LEVEL as u32),
    Protocol::new("launch-page", soma_guest::LAUNCH_PAGE_SCHEMA_VERSION as u32),
    Protocol::new(
        "launch-page-fork",
        soma_guest::FORK_LAUNCH_PAGE_SCHEMA_VERSION as u32,
    ),
];

/// The report `soma-guest-agent --version` prints.
pub fn report() -> String {
    let speaks = Speaks {
        api: None,
        protocols: &PROTOCOLS,
    };
    soma_version::report("soma-guest-agent", &Build::unknown(), &speaks)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_report_names_the_guest_protocols_and_no_commit() {
        let report = super::report();

        assert!(report.starts_with(&format!(
            "soma-guest-agent {} (unknown)\n",
            soma_version::RELEASE
        )));
        assert!(report.contains("\ncommit: unknown\n"));
        assert!(report.contains("\nprotocol.guest-application: 1\n"));
        assert!(report.contains("\nprotocol.guest-api: 3\n"));
        assert!(report.contains("\nprotocol.launch-page: 3\n"));
    }
}
