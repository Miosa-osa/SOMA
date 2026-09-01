use std::{env, net::TcpListener, path::PathBuf, process};

use soma_api::{ApiError, LocalFacade, serve};
use soma_local::{BackendSelection, LocalRuntimeConfig};

/// The address the service binds when none is given.
///
/// It binds loopback rather than every interface. This service has no finished authentication
/// scheme, so a default that exposed it to a network would be a default that leaked sandboxes.
const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// The argument the runtime re-enters this executable with to hold one machine.
///
/// It is the same word the command line uses, because the runtime names it and both binaries
/// must answer to whatever the runtime names.
const MACHINE_HOST: &str = "machine-host";

fn main() {
    process::exit(run());
}

fn run() -> i32 {
    // A machine host is not a request this service serves: it becomes the process that holds one
    // sandbox. The runtime starts one by re-entering this executable, so every binary that opens
    // a hosted runtime has to answer here, before any listener exists. Without it this service
    // can create no sandbox at all: the host it spawns would be a copy of itself parsing
    // `machine-host` as an option it does not have.
    let mut arguments = env::args().skip(1);
    if let Some(first) = arguments.next()
        && first == MACHINE_HOST
    {
        let Some(socket) = arguments.next().filter(|_| arguments.next().is_none()) else {
            eprintln!("soma-api: usage: soma-api {MACHINE_HOST} SOCKET");
            return 64;
        };
        return soma_local::host_machine(std::path::Path::new(&socket));
    }
    let Some(options) = Options::parse(env::args().skip(1)) else {
        eprintln!(
            "soma-api: usage: soma-api [--listen ADDR] [--backend auto|kvm|macos|docker] \
             [--runtime PATH] [--state-root PATH]"
        );
        return 64;
    };
    let Ok(listener) = TcpListener::bind(&options.listen) else {
        eprintln!("soma-api: could not bind {}", options.listen);
        return 74;
    };
    eprintln!("soma-api: listening on {}", options.listen);
    let open_facade = move || {
        LocalRuntimeConfig::discover(
            options.backend,
            options.runtime.clone(),
            options.state_root.clone(),
        )
        // Every route this service serves names an Instance a later request must be able to
        // reach, so the machine has to be held by a host process rather than by whichever
        // connection happened to create it. Without this the sandbox a create call returns is
        // already gone by the time the caller addresses it.
        .map(|config| config.with_hosted_machines(true))
        .and_then(LocalFacade::open)
        .map_err(|_| {
            ApiError::new(
                503,
                "backend_unavailable",
                "the local sandbox runtime could not be opened",
                true,
            )
        })
    };
    if serve(&listener, open_facade).is_err() {
        eprintln!("soma-api: the listener stopped accepting connections");
        return 74;
    }
    0
}

#[derive(Clone, Debug)]
struct Options {
    listen: String,
    backend: BackendSelection,
    runtime: Option<PathBuf>,
    state_root: Option<PathBuf>,
}

impl Options {
    /// Parses the small fixed option set this binary accepts.
    ///
    /// The options are parsed by hand rather than with an argument-parsing crate: there are four
    /// of them, none is positional, and a dependency added for four options is a dependency the
    /// whole workspace then has to keep pinned and audited.
    fn parse(arguments: impl Iterator<Item = String>) -> Option<Self> {
        let mut options = Self {
            listen: DEFAULT_LISTEN.to_owned(),
            backend: BackendSelection::Auto,
            runtime: None,
            state_root: None,
        };
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            let value = arguments.next()?;
            match argument.as_str() {
                "--listen" => options.listen = value,
                "--backend" => options.backend = backend(&value)?,
                "--runtime" => options.runtime = Some(PathBuf::from(value)),
                "--state-root" => options.state_root = Some(PathBuf::from(value)),
                _ => return None,
            }
        }
        Some(options)
    }
}

fn backend(value: &str) -> Option<BackendSelection> {
    match value {
        "auto" => Some(BackendSelection::Auto),
        "kvm" => Some(BackendSelection::Kvm),
        "macos" => Some(BackendSelection::Macos),
        "docker" => Some(BackendSelection::Docker),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Options;

    #[test]
    fn rejects_an_option_without_a_value() {
        assert!(Options::parse(["--listen".to_owned()].into_iter()).is_none());
    }

    #[test]
    fn defaults_to_loopback_when_no_address_is_given() {
        let options = Options::parse(std::iter::empty()).expect("empty arguments parse");

        assert_eq!(options.listen, "127.0.0.1:8787");
    }
}
