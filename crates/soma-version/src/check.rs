//! Runs a built binary's `--version` and holds it to the build it came from.

use std::{path::Path, process::Command};

use crate::{Build, ParsedReport, Speaks};

/// Runs `path --version` and checks that it reports `binary`, this release, the identity in
/// `build`, and exactly the contracts in `speaks`.
///
/// A test passes the [`build!`](crate::build) of its own crate. Cargo compiles the test and the
/// binary it runs in one invocation with one environment, so the two identities must agree, and
/// a binary that dropped or invented a fact fails here.
///
/// # Errors
///
/// Returns a description of the first difference, or of a run that did not succeed.
pub fn verify_binary(
    path: &Path,
    binary: &str,
    build: &Build,
    speaks: &Speaks<'_>,
) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("{} --version did not run: {error}", path.display()))?;
    if !output.status.success() || !output.stderr.is_empty() {
        return Err(format!(
            "{binary} --version exited {:?} with stderr {:?}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|_| "the report is not UTF-8")?;
    let parsed = ParsedReport::parse(&text)?;
    let expected = ParsedReport::parse(&crate::report(binary, build, speaks))?;
    if parsed == expected {
        Ok(())
    } else {
        Err(format!("{binary} reported\n{text}expected\n{expected:#?}"))
    }
}
