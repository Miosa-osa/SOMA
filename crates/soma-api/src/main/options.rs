//! The command-line options `soma-api` accepts, and their defaults.

use std::path::PathBuf;

use soma_local::BackendSelection;

/// The address the service binds when none is given.
///
/// It binds loopback rather than every interface. This service has no finished authentication
/// scheme, so a default that exposed it to a network would be a default that leaked sandboxes.
const DEFAULT_LISTEN: &str = "127.0.0.1:8787";

/// Enough preopened runtimes for a public 100-way burst plus overlapping command requests.
const DEFAULT_WORKERS: usize = 128;

/// Prevents an accidental configuration from consuming unbounded host descriptors and threads.
const MAXIMUM_WORKERS: usize = 1024;

#[derive(Clone, Debug)]
pub struct Options {
    pub listen: String,
    pub backend: BackendSelection,
    pub runtime: Option<PathBuf>,
    pub state_root: Option<PathBuf>,
    pub workers: usize,
    /// The public runner's configuration file; the runner is off without one.
    pub runner_config: Option<PathBuf>,
}

impl Options {
    /// Parses the small fixed option set this binary accepts.
    ///
    /// The options are parsed by hand rather than with an argument-parsing crate: there are four
    /// of them, none is positional, and a dependency added for four options is a dependency the
    /// whole workspace then has to keep pinned and audited.
    pub fn parse(arguments: impl Iterator<Item = String>) -> Option<Self> {
        let mut options = Self {
            listen: DEFAULT_LISTEN.to_owned(),
            backend: BackendSelection::Auto,
            runtime: None,
            state_root: None,
            workers: DEFAULT_WORKERS,
            runner_config: None,
        };
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            let value = arguments.next()?;
            match argument.as_str() {
                "--listen" => options.listen = value,
                "--backend" => options.backend = backend(&value)?,
                "--runtime" => options.runtime = Some(PathBuf::from(value)),
                "--state-root" => options.state_root = Some(PathBuf::from(value)),
                "--runner-config" => options.runner_config = Some(PathBuf::from(value)),
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
        assert!(
            options.runner_config.is_none(),
            "the public runner is off by default"
        );
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
