#![doc = "The release version and source identity every SOMA binary reports."]
#![forbid(unsafe_code)]
//!
//! One number names a SOMA release: [`RELEASE`], the Cargo workspace version, which
//! `scripts/check-version.sh` holds equal to `VERSION` and every crate.
//! The commit a binary was built from reaches it through [`build!`], which the binary's own
//! crate expands, so a new commit recompiles only the crates that print it.
//! `scripts/build-identity.sh` sets the commit, its date, and whether the tree was clean; a
//! binary built without it reports all three as `unknown` rather than guessing.

mod check;
mod report;

pub use check::verify_binary;
pub use report::{BUILD_KEYS, ParsedReport, Protocol, Speaks, report};

/// The release version of the whole workspace, shared by every crate.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");

/// The Cargo profile the binary was compiled with, `release` or `debug`.
pub const PROFILE: &str = env!("SOMA_VERSION_PROFILE");

/// The Rust target triple the binary was compiled for.
pub const TARGET: &str = env!("SOMA_VERSION_TARGET");

/// How many hexadecimal digits the short commit keeps.
pub const SHORT_COMMIT_DIGITS: usize = 12;

/// The environment variable that carries the full source commit into a build.
pub const COMMIT_ENV: &str = "SOMA_BUILD_COMMIT";

/// The environment variable that carries the commit's own date, `YYYY-MM-DDTHH:MM:SSZ`.
pub const COMMIT_DATE_ENV: &str = "SOMA_BUILD_COMMIT_DATE";

/// The environment variable that says whether the tree matched the commit: `clean` or `dirty`.
pub const SOURCE_ENV: &str = "SOMA_BUILD_SOURCE";

/// The words a report prints when a fact was not given to the build.
pub const UNKNOWN: &str = "unknown";

/// Captures the build identity the calling crate was compiled with.
///
/// It expands in the caller, so `option_env!` reads the environment of the caller's own
/// compilation and Cargo recompiles only that crate when the commit changes.
#[macro_export]
macro_rules! build {
    () => {
        $crate::Build::from_environment(
            option_env!("SOMA_BUILD_COMMIT"),
            option_env!("SOMA_BUILD_COMMIT_DATE"),
            option_env!("SOMA_BUILD_SOURCE"),
        )
    };
}

/// Whether the working tree a binary was built from matched its commit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    /// Every tracked and untracked file matched the commit.
    Clean,
    /// Some file differed from the commit, so the commit alone does not reproduce the binary.
    Dirty,
    /// The build was not told which commit it came from.
    Unknown,
}

impl Source {
    /// The word a report prints for this state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Dirty => "dirty",
            Self::Unknown => UNKNOWN,
        }
    }
}

/// The identity of one build: what was compiled, from which commit, for which target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Build {
    commit: Option<&'static str>,
    commit_date: Option<&'static str>,
    source: Source,
}

impl Build {
    /// Builds the identity from the three values a build was given, keeping only valid ones.
    ///
    /// A commit must be 40 or 64 lowercase hexadecimal digits, a date must be the UTC form the
    /// identity script writes, and a source must be `clean` or `dirty`. Anything else is
    /// treated as absent, and without a commit nothing else is claimed either.
    #[must_use]
    pub fn from_environment(
        commit: Option<&'static str>,
        commit_date: Option<&'static str>,
        source: Option<&'static str>,
    ) -> Self {
        let Some(commit) = commit.filter(|commit| is_commit(commit)) else {
            return Self::unknown();
        };
        let source = match source {
            Some("clean") => Source::Clean,
            Some("dirty") => Source::Dirty,
            _ => Source::Unknown,
        };
        Self {
            commit: Some(commit),
            commit_date: commit_date.filter(|date| is_utc_timestamp(date)),
            source,
        }
    }

    /// An identity that claims nothing beyond the release version, profile, and target.
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            commit: None,
            commit_date: None,
            source: Source::Unknown,
        }
    }

    /// The release version.
    #[must_use]
    pub const fn release(&self) -> &'static str {
        RELEASE
    }

    /// The full source commit, or `unknown`.
    #[must_use]
    pub fn commit(&self) -> &'static str {
        self.commit.unwrap_or(UNKNOWN)
    }

    /// The first [`SHORT_COMMIT_DIGITS`] digits of the commit, or `unknown`.
    #[must_use]
    pub fn commit_short(&self) -> &'static str {
        self.commit
            .map_or(UNKNOWN, |commit| &commit[..SHORT_COMMIT_DIGITS])
    }

    /// The commit's own date in UTC, `YYYY-MM-DDTHH:MM:SSZ`, or `unknown`.
    #[must_use]
    pub fn commit_date(&self) -> &'static str {
        self.commit_date.unwrap_or(UNKNOWN)
    }

    /// Whether the tree matched the commit.
    #[must_use]
    pub const fn source(&self) -> Source {
        self.source
    }

    /// The short commit, marked `-dirty` when the tree differed from it.
    #[must_use]
    pub fn revision(&self) -> String {
        match self.source {
            Source::Dirty => format!("{}-dirty", self.commit_short()),
            Source::Clean | Source::Unknown => self.commit_short().to_owned(),
        }
    }

    /// The profile the binary was compiled with.
    #[must_use]
    pub const fn profile(&self) -> &'static str {
        PROFILE
    }

    /// The target triple the binary was compiled for.
    #[must_use]
    pub const fn target(&self) -> &'static str {
        TARGET
    }
}

/// Whether the arguments, program name first, ask only for the version report.
///
/// Exactly one argument, `--version`, is a version request. Anything else is left to the
/// binary's own parser, which refuses `--version` mixed with other options as it refuses any
/// other unexpected word.
#[must_use]
pub fn version_requested<I, S>(arguments: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut arguments = arguments.into_iter().skip(1);
    matches!(
        (arguments.next(), arguments.next()),
        (Some(first), None) if first.as_ref() == "--version"
    )
}

fn is_commit(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_utc_timestamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests;
