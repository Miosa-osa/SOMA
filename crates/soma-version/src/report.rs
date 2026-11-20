//! The `--version` report: one line for people, then one `key: value` line per fact.
//!
//! The keys are a stable contract. The release packager reads them to write the release
//! manifest, so a binary's report and the manifest entry for it come from the same bytes.

use std::fmt::Write as _;

use crate::Build;

/// One wire or file protocol a binary speaks, with the version this build speaks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Protocol {
    /// The protocol's name, as `docs/versioning.md` lists it.
    pub name: &'static str,
    /// The version this build speaks.
    pub version: u32,
}

impl Protocol {
    /// Names one protocol version.
    #[must_use]
    pub const fn new(name: &'static str, version: u32) -> Self {
        Self { name, version }
    }
}

/// The versioned contracts one binary speaks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Speaks<'a> {
    /// The HTTP API version, for a binary that serves or calls the SOMA API.
    pub api: Option<&'a str>,
    /// Every protocol the binary speaks, in the order it prints them.
    pub protocols: &'a [Protocol],
}

/// The build keys every report carries, in the order it prints them.
pub const BUILD_KEYS: [&str; 7] = [
    "release",
    "commit",
    "commit-short",
    "commit-date",
    "source",
    "profile",
    "target",
];

/// Renders the version report for `binary`.
///
/// The first line reads like `soma-api 1.0.0 (2ec3789c4f1a 2026-11-20)`. The lines after it
/// are `release`, `commit`, `commit-short`, `commit-date`, `source`, `profile`, `target`, then
/// `api` when the binary speaks it, then one `protocol.<name>` line per protocol.
#[must_use]
pub fn report(binary: &str, build: &Build, speaks: &Speaks<'_>) -> String {
    let mut text = String::new();
    let day = build.commit_date().get(..10).unwrap_or(crate::UNKNOWN);
    let headline = if build.commit_date() == crate::UNKNOWN {
        build.revision()
    } else {
        format!("{} {day}", build.revision())
    };
    // Writing to a `String` cannot fail, so every result below is infallible.
    let _ = writeln!(text, "{binary} {} ({headline})", build.release());
    for (key, value) in BUILD_KEYS.into_iter().zip([
        build.release(),
        build.commit(),
        build.commit_short(),
        build.commit_date(),
        build.source().as_str(),
        build.profile(),
        build.target(),
    ]) {
        let _ = writeln!(text, "{key}: {value}");
    }
    if let Some(api) = speaks.api {
        let _ = writeln!(text, "api: {api}");
    }
    for protocol in speaks.protocols {
        let _ = writeln!(text, "protocol.{}: {}", protocol.name, protocol.version);
    }
    text
}

/// A version report read back: what a client or a test learns from `--version`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedReport {
    /// The binary's name from the first line.
    pub binary: String,
    /// The values of [`BUILD_KEYS`], in that order.
    pub build: [String; 7],
    /// The API version, when the binary speaks one.
    pub api: Option<String>,
    /// Every protocol, by name, in the order printed.
    pub protocols: Vec<(String, u32)>,
}

impl ParsedReport {
    /// Reads a report [`report`] wrote, refusing any other shape.
    ///
    /// # Errors
    ///
    /// Returns which part of the report is missing or malformed.
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let mut lines = text.lines();
        let headline = lines.next().ok_or("the report is empty")?;
        let binary = headline
            .split_once(' ')
            .map(|(binary, _)| binary.to_owned())
            .ok_or("the first line names no binary")?;
        let mut build: [String; 7] = Default::default();
        for (slot, key) in build.iter_mut().zip(BUILD_KEYS) {
            let line = lines.next().ok_or("a build key is missing")?;
            line.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(": "))
                .ok_or("a build key is out of order")?
                .clone_into(slot);
        }
        let mut api = None;
        let mut protocols = Vec::new();
        for line in lines {
            let (key, value) = line.split_once(": ").ok_or("a line is not key: value")?;
            if key == "api" && api.is_none() && protocols.is_empty() {
                api = Some(value.to_owned());
            } else if let Some(name) = key.strip_prefix("protocol.") {
                let version = value
                    .parse()
                    .map_err(|_| "a protocol version is not a number")?;
                protocols.push((name.to_owned(), version));
            } else {
                return Err("an unknown key follows the build keys");
            }
        }
        Ok(Self {
            binary,
            build,
            api,
            protocols,
        })
    }

    /// The value of one of [`BUILD_KEYS`].
    #[must_use]
    pub fn value(&self, key: &str) -> Option<&str> {
        BUILD_KEYS
            .iter()
            .position(|known| *known == key)
            .map(|index| self.build[index].as_str())
    }
}
