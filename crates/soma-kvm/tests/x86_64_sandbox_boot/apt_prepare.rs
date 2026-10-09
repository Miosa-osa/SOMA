//! What the image carries for the workload's `prepare()`, proven inside a live large-shape guest.
//!
//! The workload runs `apt-get update` unconditionally before installing a fixed package list, and
//! its published totals include that phase. Everything an image can do about that is already done
//! here, and this gate proves the half that does not need a network: every package the install
//! names is in the image, the sources are trimmed to the suite and component those packages live
//! in, and the install therefore has nothing to do.
//!
//! The other half, what the update costs when it reaches the archive, cannot be measured here. A
//! machine booted by this harness has no egress: the device layer puts a link-down placeholder
//! behind the network device because there is no TAP broker in a test process, so every attempt to
//! reach `archive.ubuntu.com` from this guest fails immediately and `apt-get update` spends about
//! seven seconds retrying and then errors. That number is an artefact of the harness and is
//! reported as such rather than asserted. The real figure comes from a sandbox launched by the
//! runner, which is where the workload runs.

use crate::{
    live::{boot_generation, serialize_live_proof},
    x86_64_sandbox_boot_generation as generation,
    x86_64_sandbox_boot_host::{assert_proof, require_kvm},
    x86_64_sandbox_boot_session as session,
};

/// The image the workload is built from.
const LARGE: &str = "soma-large-dax:3";
/// The machine contract v2 shape the workload runs at.
const MEMORY_MIB: u64 = 16 * 1024;
const STORAGE_MIB: u64 = 20 * 1024;
const VCPUS: u16 = 8;

/// The one command the gate runs, reported as `key=value` lines.
///
/// Nothing here reaches the network: the harness gives this guest no egress, so an apt operation
/// that needs the archive reports the harness rather than the image.
pub const SCRIPT: &str = r#"set -u
echo "SOURCES=$(grep -h '^Suites:' /etc/apt/sources.list.d/*.sources 2>/dev/null | tr '\n' ';')"
echo "COMPONENTS=$(grep -h '^Components:' /etc/apt/sources.list.d/*.sources 2>/dev/null | tr '\n' ';')"
echo "DEB_SRC=$(grep -ch 'deb-src' /etc/apt/sources.list.d/*.sources 2>/dev/null | tr '\n' ';')"
echo "LISTS_KIB=$(du -sk /var/lib/apt/lists | cut -f1)"
for package in bash build-essential ca-certificates curl git python3 python3-setuptools unzip; do
  echo "PKG_${package}=$(dpkg-query -W -f='${Status}' "$package" 2>/dev/null || echo missing)"
done
echo "NODE=$(command -v node >/dev/null 2>&1 && echo present || echo absent)"
start=$(date +%s%N)
DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-download \
  bash build-essential ca-certificates curl git python3 python3-setuptools unzip 2>/dev/null
echo "INSTALL_RC=$?"
echo "INSTALL_MS=$(( ($(date +%s%N) - start) / 1000000 ))"
echo "NEWLY_INSTALLED=$(apt-get install -y -qq --no-download --print-uris \
  bash build-essential ca-certificates curl git python3 python3-setuptools unzip 2>/dev/null | grep -c '^.' )"
echo END"#;

#[test]
#[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and the large OCI layout"]
fn the_large_shape_already_carries_the_packages_prepare_installs() {
    let _serialized = serialize_live_proof();
    require_kvm();
    let command = session::Command {
        program: b"/bin/bash",
        arguments: &[b"-c", SCRIPT.as_bytes()],
        // Two offline apt operations and nothing that waits on a network this guest does not have.
        timeout_millis: 120_000,
        output_bytes: 1 << 20,
    };
    let proof = boot_generation(
        "large",
        LARGE,
        "SOMA_OCI_LARGE_LAYOUT",
        generation::Shape::new(MEMORY_MIB, STORAGE_MIB, VCPUS),
        &command,
    )
    .expect("prerequisite failed: the large OCI layout could not be exported; set SOMA_OCI_LARGE_LAYOUT");
    assert_proof(&proof);
    let stdout = String::from_utf8_lossy(&proof.executed.stdout);
    eprintln!("[{LARGE}] prepare report:\n{stdout}");
    assert_prepare(&stdout);
}

/// Asserts what an image can be held to without a network.
fn assert_prepare(stdout: &str) {
    assert!(stdout.contains("END"), "the guest script did not finish");
    // The sources are the trimmed set, so the update cannot fetch an index nothing needs. The
    // reported value is the whole line `grep` printed, key included.
    assert!(
        line(stdout, "sources").is_some_and(|s| s.contains("noble") && s.contains("noble-updates")),
        "stdout={stdout:?}"
    );
    assert_eq!(
        line(stdout, "components"),
        Some("Components: main;"),
        "stdout={stdout:?}"
    );
    assert!(
        line(stdout, "deb_src").is_some_and(|count| count.trim_matches(';') == "0"),
        "a source list still carries deb-src: stdout={stdout:?}"
    );
    // The lists the image carries are what let an update answer from disk rather than download.
    assert!(
        reported(stdout, "lists_kib").is_some_and(|kib| kib > 1024),
        "the image ships no apt index lists: stdout={stdout:?}"
    );
    // Every package the workload's install names is already here, so the install is a no-op: it
    // succeeds without a network and names nothing to download.
    for package in [
        "bash",
        "build-essential",
        "ca-certificates",
        "curl",
        "git",
        "python3",
        "python3-setuptools",
        "unzip",
    ] {
        assert_eq!(
            line(stdout, &format!("pkg_{package}")),
            Some("install ok installed"),
            "{package} is not in the image: stdout={stdout:?}"
        );
    }
    assert_eq!(line(stdout, "node"), Some("present"), "stdout={stdout:?}");
    assert_eq!(
        reported(stdout, "install_rc"),
        Some(0),
        "the install did not succeed with nothing to download: stdout={stdout:?}"
    );
    assert_eq!(
        reported(stdout, "newly_installed"),
        Some(0),
        "the install still has something to fetch: stdout={stdout:?}"
    );
    assert!(
        reported(stdout, "install_ms").is_some_and(|ms| ms < 10_000),
        "the install did not finish promptly: stdout={stdout:?}"
    );
}

/// Reads one `key=value` line as its value, matching the key without regard to case.
fn line<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        name.eq_ignore_ascii_case(key).then_some(value.trim())
    })
}

/// Reads one `key=value` line as a number.
fn reported(stdout: &str, key: &str) -> Option<u64> {
    line(stdout, key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reader_finds_the_packages_and_the_timings() {
        let report = "SOURCES=noble noble-updates;\nCOMPONENTS=main;\nDEB_SRC=0;\n\
                      LISTS_KIB=6060\nPKG_python3-setuptools=install ok installed\n\
                      UPDATE_1_MS=812\nINSTALL_MS=306\nEND\n";
        assert!(line(report, "sources").is_some_and(|s| s.contains("noble-updates")));
        assert_eq!(line(report, "components"), Some("main;"));
        assert_eq!(
            line(report, "pkg_python3-setuptools"),
            Some("install ok installed")
        );
        assert_eq!(reported(report, "update_1_ms"), Some(812));
        assert_eq!(reported(report, "lists_kib"), Some(6060));
        assert_eq!(line(report, "end"), None);
    }
}
