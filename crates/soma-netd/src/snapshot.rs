//! A periodic per-bundle egress snapshot for the control plane's usage metering.
//!
//! Off unless `SOMA_NETD_USAGE_SNAPSHOT` names a file. While it is set the daemon reads, every
//! `SOMA_NETD_USAGE_SNAPSHOT_SECS` (default 30, at least 5), the sandbox veth counters of each
//! assigned and unreleased bundle (see [`crate::usage`]: the counters sit behind the forwarding
//! chain's drops, so they are the bytes the guest sent to where policy allows) and replaces the
//! file atomically. The counters live in the kernel with the bundle, so they keep counting across
//! a daemon restart; a reader tells a replaced bundle from a continuing one by its `bundle` id.
//!
//! The file is read-only evidence for a reader on the same host. Nothing here decides or changes
//! traffic, and a failed read of one bundle only leaves that bundle out of this snapshot.

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{Broker, BundleNames, namespace::NetNamespace, usage};

const PATH_ENV: &str = "SOMA_NETD_USAGE_SNAPSHOT";
const SECS_ENV: &str = "SOMA_NETD_USAGE_SNAPSHOT_SECS";
const DEFAULT_SECS: u64 = 30;
const MIN_SECS: u64 = 5;

/// Where and how often to write the snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Config {
    path: PathBuf,
    interval: Duration,
}

impl Config {
    /// Reads the configuration from the environment; `None` leaves the snapshot off.
    pub(crate) fn from_env() -> Option<Self> {
        Self::parse(
            std::env::var(PATH_ENV).ok().as_deref(),
            std::env::var(SECS_ENV).ok().as_deref(),
        )
    }

    fn parse(path: Option<&str>, secs: Option<&str>) -> Option<Self> {
        let path = path.map(str::trim).filter(|path| !path.is_empty())?;
        let secs = secs
            .and_then(|secs| secs.trim().parse::<u64>().ok())
            .map_or(DEFAULT_SECS, |secs| secs.max(MIN_SECS));
        Some(Self {
            path: PathBuf::from(path),
            interval: Duration::from_secs(secs),
        })
    }
}

/// One bundle's counters at one instant.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Sample {
    instance: String,
    bundle: String,
    bytes_out: u64,
    packets_out: u64,
}

/// Writes the snapshot every interval, for the life of the process.
pub(crate) fn run(broker: &Broker, config: &Config) {
    loop {
        std::thread::sleep(config.interval);
        if let Err(error) = write(&config.path, &render(&collect(broker), now_millis())) {
            eprintln!(
                "soma-netd: usage snapshot {} not written: {error}",
                config.path.display()
            );
        }
    }
}

fn collect(broker: &Broker) -> Vec<Sample> {
    let Ok(entries) = broker.ledger().entries() else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter(|entry| !entry.released)
        .filter_map(|entry| {
            let record = entry.record;
            let short = record.bundle.short_hex();
            let names = BundleNames::new(&short);
            let namespace = NetNamespace::open(&broker.namespace_dir().join(&short)).ok()?;
            let counted = usage::read(&namespace, &names.sandbox_veth).ok()?;
            Some(Sample {
                instance: hex(record.instance.as_bytes()),
                bundle: short,
                bytes_out: counted.bytes_out,
                packets_out: counted.packets_out,
            })
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

/// The snapshot as JSON; every value is a number or a lowercase hex string, so nothing needs escaping.
fn render(samples: &[Sample], sampled_at_unix_ms: u64) -> String {
    let bundles = samples
        .iter()
        .map(|sample| {
            format!(
                "{{\"instance\":\"{}\",\"bundle\":\"{}\",\"bytes_out\":{},\"packets_out\":{}}}",
                sample.instance, sample.bundle, sample.bytes_out, sample.packets_out
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"version\":1,\"sampled_at_unix_ms\":{sampled_at_unix_ms},\"bundles\":[{bundles}]}}\n"
    )
}

/// Replaces `path` with `text` so a reader sees the old file or the new one, never half of one.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    let staging = path.with_extension("tmp");
    fs::write(&staging, text)?;
    fs::rename(&staging, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(instance: &str, bundle: &str, bytes_out: u64) -> Sample {
        Sample {
            instance: instance.to_owned(),
            bundle: bundle.to_owned(),
            bytes_out,
            packets_out: 3,
        }
    }

    #[test]
    fn the_snapshot_is_off_without_a_path() {
        assert_eq!(Config::parse(None, Some("10")), None);
        assert_eq!(Config::parse(Some("  "), None), None);
    }

    #[test]
    fn the_interval_defaults_and_has_a_floor() {
        let default = Config::parse(Some("/run/u.json"), None).expect("on");
        assert_eq!(default.interval, Duration::from_secs(30));
        let floor = Config::parse(Some("/run/u.json"), Some("1")).expect("on");
        assert_eq!(floor.interval, Duration::from_secs(5));
        let junk = Config::parse(Some("/run/u.json"), Some("soon")).expect("on");
        assert_eq!(junk.interval, Duration::from_secs(30));
        let set = Config::parse(Some("/run/u.json"), Some("60")).expect("on");
        assert_eq!(set.interval, Duration::from_secs(60));
    }

    #[test]
    fn samples_render_as_one_json_document() {
        let text = render(
            &[
                sample("0a0b0c0d0e0f00112233445566778899", "01020304", 123),
                sample("ffeeddccbbaa99887766554433221100", "05060708", 0),
            ],
            1_700_000_000_000,
        );
        assert_eq!(
            text,
            "{\"version\":1,\"sampled_at_unix_ms\":1700000000000,\"bundles\":[\
             {\"instance\":\"0a0b0c0d0e0f00112233445566778899\",\"bundle\":\"01020304\",\"bytes_out\":123,\"packets_out\":3},\
             {\"instance\":\"ffeeddccbbaa99887766554433221100\",\"bundle\":\"05060708\",\"bytes_out\":0,\"packets_out\":3}]}\n"
        );
    }

    #[test]
    fn no_bundles_still_renders_a_valid_document() {
        assert_eq!(
            render(&[], 5),
            "{\"version\":1,\"sampled_at_unix_ms\":5,\"bundles\":[]}\n"
        );
    }

    #[test]
    fn the_file_is_replaced_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("usage.json");
        write(&path, "first\n").expect("first");
        write(&path, "second\n").expect("second");
        assert_eq!(fs::read_to_string(&path).expect("read"), "second\n");
        assert!(!dir.path().join("usage.tmp").exists());
    }

    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x0a, 0xff, 0x00]), "0aff00");
    }
}
