//! The real ext4 operations a host runs on a detached volume file.
//!
//! Creating one is the ext4 maker on a sparse file of the volume's class; repairing one is
//! `e2fsck` under the automatic policy. Both are the offline half of an attach: the store runs
//! them under the volume lock, before any machine holds the file, and the receipt carries what
//! happened, so a customer can be told when their data was repaired and why.

use std::{
    ffi::OsString,
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::Command,
};

use crate::VolumeId;

use super::record::now_nanos;
use super::repair::{
    CreateError, FORMAT_TOOL, REPAIR_TOOL, RepairOutcome, RepairReceipt, VolumeRepair,
};

/// The ext4 tools one host runs, named by the paths it runs them from.
pub struct Ext4 {
    fsck: PathBuf,
    make: PathBuf,
}

impl Ext4 {
    /// The tools as `PATH` names them.
    #[must_use]
    pub fn from_path() -> Self {
        Self::at(REPAIR_TOOL, FORMAT_TOOL)
    }

    /// The tools at exact paths, which is how a host that keeps them outside `PATH` names them.
    #[must_use]
    pub fn at(fsck: impl Into<PathBuf>, make: impl Into<PathBuf>) -> Self {
        Self {
            fsck: fsck.into(),
            make: make.into(),
        }
    }

    /// The argument vector one repair runs: a forced check under the automatic policy.
    ///
    /// The policy is `-p`, never `-y`: a preen repairs what is unambiguously damaged and leaves
    /// everything else for a human, which is the behavior the repair receipt reports.
    #[must_use]
    pub fn fsck_argv(&self, file: &Path) -> Vec<OsString> {
        vec!["-f".into(), "-p".into(), file.into()]
    }

    /// The argument vector one create runs, on a file the caller has already sized.
    #[must_use]
    pub fn make_argv(&self, file: &Path) -> Vec<OsString> {
        vec![
            "-q".into(),
            "-F".into(),
            "-t".into(),
            "ext4".into(),
            file.into(),
        ]
    }

    /// Sizes the path for creation, or refuses it because it already holds data.
    ///
    /// Only a path that does not exist yet is created, and the guard runs before the call that
    /// sets the length, so a path that already holds data is somebody's volume and is never
    /// opened in a mode that could empty it.
    fn size(file: &Path, size_bytes: u64) -> Result<(), CreateError> {
        if size_bytes == 0 {
            return Err(CreateError::Size(size_bytes));
        }
        let handle = match OpenOptions::new().read(true).write(true).open(file) {
            Ok(existing) => existing,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(file)
                .map_err(|error| tool_error(&error))?,
            Err(error) => return Err(tool_error(&error)),
        };
        if handle.metadata().map_err(|error| tool_error(&error))?.len() > 0 {
            return Err(CreateError::Occupied);
        }
        handle
            .set_len(size_bytes)
            .map_err(|error| tool_error(&error))?;
        Ok(())
    }
}

/// The exit code of a failed system call, or -1 when the kernel did not give one.
fn tool_error(error: &std::io::Error) -> CreateError {
    CreateError::Tool(error.raw_os_error().unwrap_or(-1))
}

/// The automatic policy's reading of one `e2fsck` exit code.
///
/// 0 is a filesystem that was already consistent, 1 and 2 are ones this run corrected, 4 is
/// damage the preen policy will not repair without a human, and everything else - an operational
/// error, a usage error, a cancellation, or a tool that could not be run at all - is a failure
/// the receipt states rather than hides.
#[must_use]
pub const fn outcome_from_exit(code: Option<i32>) -> RepairOutcome {
    match code {
        Some(0) => RepairOutcome::Clean,
        Some(1 | 2) => RepairOutcome::Repaired,
        Some(4) => RepairOutcome::Refused,
        _ => RepairOutcome::Failed,
    }
}

/// The exit code as the receipt carries it, saturated into a signed byte.
fn exit_byte(code: Option<i32>) -> i8 {
    match code {
        None => -1,
        Some(value) => i8::try_from(value).unwrap_or(if value > 0 { i8::MAX } else { i8::MIN }),
    }
}

impl VolumeRepair for Ext4 {
    fn create(&self, _volume: VolumeId, file: &Path, size_bytes: u64) -> Result<(), CreateError> {
        Self::size(file, size_bytes)?;
        let status = Command::new(&self.make)
            .args(self.make_argv(file))
            .status()
            .map_err(|error| tool_error(&error))?;
        if status.success() {
            Ok(())
        } else {
            Err(CreateError::Tool(status.code().unwrap_or(-1)))
        }
    }

    fn repair(&self, volume: VolumeId, file: &Path) -> RepairReceipt {
        let code = Command::new(&self.fsck)
            .args(self.fsck_argv(file))
            .status()
            .ok()
            .and_then(|status| status.code());
        RepairReceipt::new(
            volume,
            outcome_from_exit(code),
            exit_byte(code),
            now_nanos(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume() -> VolumeId {
        VolumeId::new([9; 16]).expect("nonzero")
    }

    fn tools(directory: &Path) -> Ext4 {
        // Paths that cannot be executed, so every test that reaches a tool states the failure
        // rather than running one.
        Ext4::at(directory.join("no-tool"), directory.join("no-tool"))
    }

    #[test]
    fn every_fsck_exit_code_maps_to_the_outcome_the_policy_means() {
        assert_eq!(outcome_from_exit(Some(0)), RepairOutcome::Clean);
        assert_eq!(outcome_from_exit(Some(1)), RepairOutcome::Repaired);
        assert_eq!(outcome_from_exit(Some(2)), RepairOutcome::Repaired);
        assert_eq!(outcome_from_exit(Some(4)), RepairOutcome::Refused);
        for code in [Some(8), Some(16), Some(32), Some(-1), None] {
            assert_eq!(outcome_from_exit(code), RepairOutcome::Failed);
        }
        assert!(outcome_from_exit(Some(0)).is_usable());
        assert!(!outcome_from_exit(Some(4)).is_usable());
    }

    #[test]
    fn the_receipt_carries_the_exit_code_saturated_and_never_a_panic() {
        assert_eq!(exit_byte(Some(0)), 0);
        assert_eq!(exit_byte(Some(4)), 4);
        assert_eq!(exit_byte(Some(4000)), i8::MAX);
        assert_eq!(exit_byte(None), -1);
    }

    #[test]
    fn the_repair_is_a_forced_check_under_the_automatic_policy() {
        let directory = tempfile::tempdir().expect("temp dir");
        let tools = tools(directory.path());
        let file = directory.path().join("volume.img");
        let argv: Vec<String> = tools
            .fsck_argv(&file)
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            argv,
            vec![
                "-f".to_owned(),
                "-p".to_owned(),
                file.to_string_lossy().into_owned()
            ]
        );
        assert!(!argv.iter().any(|argument| argument == "-y"));

        let make: Vec<String> = tools
            .make_argv(&file)
            .into_iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(make[0], "-q");
        assert!(make.contains(&"-F".to_owned()));
        assert!(make.contains(&"ext4".to_owned()));
    }

    #[test]
    fn a_create_refuses_a_path_that_already_holds_data() {
        let directory = tempfile::tempdir().expect("temp dir");
        let tools = tools(directory.path());
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"someone's filesystem").expect("write");
        assert_eq!(
            tools.create(volume(), &file, 1 << 20),
            Err(CreateError::Occupied)
        );
        assert_eq!(
            std::fs::read(&file).expect("read"),
            b"someone's filesystem",
            "the guard must run before the file is sized"
        );
    }

    #[test]
    fn a_create_refuses_a_size_a_filesystem_cannot_have() {
        let directory = tempfile::tempdir().expect("temp dir");
        let tools = tools(directory.path());
        let file = directory.path().join("volume.img");
        assert_eq!(tools.create(volume(), &file, 0), Err(CreateError::Size(0)));
        assert!(
            !file.exists(),
            "nothing is created for a size that is refused"
        );
    }

    #[test]
    fn a_create_sizes_an_empty_file_and_states_a_tool_it_cannot_run() {
        let directory = tempfile::tempdir().expect("temp dir");
        let tools = tools(directory.path());
        let file = directory.path().join("volume.img");
        std::fs::write(&file, b"").expect("empty file");
        assert!(matches!(
            tools.create(volume(), &file, 1 << 20),
            Err(CreateError::Tool(_))
        ));
        // The file was sized before the tool ran, so the failure is the tool's and not the shape's.
        assert_eq!(std::fs::metadata(&file).expect("metadata").len(), 1 << 20);
    }

    #[test]
    fn a_repair_that_cannot_run_is_a_stated_failure_and_never_a_silent_clean() {
        let directory = tempfile::tempdir().expect("temp dir");
        let tools = tools(directory.path());
        let receipt = tools.repair(volume(), &directory.path().join("volume.img"));
        assert_eq!(receipt.outcome, RepairOutcome::Failed);
        assert!(!receipt.is_usable());
        assert!(receipt.to_string().contains(REPAIR_TOOL));
    }
}
