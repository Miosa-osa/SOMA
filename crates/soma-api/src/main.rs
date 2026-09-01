use std::{env, net::TcpListener, path::PathBuf, process, time::Duration};

use soma_api::{ApiError, FacadePool, LocalFacade, serve};
use soma_local::{BackendSelection, LocalRuntimeConfig};

/// The address the service binds when none is given.
///
/// It binds loopback rather than every interface. This service has no finished authentication
/// scheme, so a default that exposed it to a network would be a default that leaked sandboxes.
const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// Enough preopened runtimes for a public 100-way burst plus overlapping command requests.
const DEFAULT_WORKERS: usize = 128;

/// Prevents an accidental configuration from consuming unbounded host descriptors and threads.
const MAXIMUM_WORKERS: usize = 1024;
const POOL_WAIT_TIMEOUT: Duration = Duration::from_secs(1);

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
        if arguments.next().is_some() {
            eprintln!("soma-api: usage: soma-api {MACHINE_HOST}");
            return 64;
        }
        return soma_local::host_machine(None);
    }
    let Some(options) = Options::parse(env::args().skip(1)) else {
        eprintln!(
            "soma-api: usage: soma-api [--listen ADDR] [--backend auto|kvm|macos|docker] \
             [--runtime PATH] [--state-root PATH] [--workers COUNT]"
        );
        return 64;
    };
    let Ok(listener) = TcpListener::bind(&options.listen) else {
        eprintln!("soma-api: could not bind {}", options.listen);
        return 74;
    };
    let Ok(config) = LocalRuntimeConfig::discover(
        options.backend,
        options.runtime.clone(),
        options.state_root.clone(),
    ) else {
        eprintln!("soma-api: the local sandbox runtime configuration is invalid");
        return 78;
    };
    let config = config.with_hosted_machines(true);
    if matches!(
        options.backend,
        BackendSelection::Auto | BackendSelection::Kvm
    ) && soma_local::prewarm_machine_hosts(options.workers).is_err()
    {
        eprintln!("soma-api: sterile machine hosts could not be prepared");
        return 69;
    }
    let Ok(pool) = FacadePool::open(options.workers, || LocalFacade::open(config.clone())) else {
        eprintln!("soma-api: the local sandbox runtime could not be opened");
        return 69;
    };
    eprintln!("soma-api: listening on {}", options.listen);
    let open_facade = move || {
        pool.acquire_timeout(POOL_WAIT_TIMEOUT).ok_or_else(|| {
            ApiError::new(
                503,
                "runtime_busy",
                "the local sandbox runtime is at its bounded request capacity",
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
    workers: usize,
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
            workers: DEFAULT_WORKERS,
        };
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            let value = arguments.next()?;
            match argument.as_str() {
                "--listen" => options.listen = value,
                "--backend" => options.backend = backend(&value)?,
                "--runtime" => options.runtime = Some(PathBuf::from(value)),
                "--state-root" => options.state_root = Some(PathBuf::from(value)),
                "--workers" => {
                    options.workers = value.parse().ok()?;
                    if !(1..=MAXIMUM_WORKERS).contains(&options.workers) {
                        return None;
                    }
                }
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
        assert_eq!(options.workers, 128);
    }

    #[test]
    fn accepts_a_bounded_worker_count() {
        let options = Options::parse(["--workers".to_owned(), "100".to_owned()].into_iter())
            .expect("bounded worker count parses");

        assert_eq!(options.workers, 100);
    }

    #[test]
    fn rejects_zero_or_unbounded_worker_counts() {
        for count in ["0", "1025", "not-a-number"] {
            assert!(
                Options::parse(["--workers".to_owned(), count.to_owned()].into_iter()).is_none()
            );
        }
    }
}
