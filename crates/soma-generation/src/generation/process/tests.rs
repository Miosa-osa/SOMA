//! Containment gates for one external build tool.
//!
//! The fixtures are ordinary shell programs, so they exercise the real spawn, group, deadline,
//! termination, and collection paths rather than a stub.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::{
    CompileError, CompileErrorKind, CompilePhase, Invocation, PinnedTool, TERMINATION_GRACE,
};

const SHELL: &str = "/bin/sh";
const DEADLINE: Duration = Duration::from_millis(300);

mod pinned;

/// Descriptor accounting is process wide, so the spawning tests take turns.
static SPAWNING: Mutex<()> = Mutex::new(());

fn serialized() -> MutexGuard<'static, ()> {
    SPAWNING.lock().unwrap_or_else(PoisonError::into_inner)
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("soma-process-containment-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("scratch directory");
    directory.join(name)
}

fn pinned(program: &str, phase: CompilePhase) -> PinnedTool {
    PinnedTool::open(Path::new(program), phase).expect("a pinned host tool")
}

fn shell(script: &str, phase: CompilePhase, deadline: Duration) -> Result<(), CompileError> {
    Invocation {
        program: &pinned(SHELL, phase),
        arguments: vec![OsString::from("-c"), OsString::from(script)],
        environment: Vec::new(),
        working_directory: Path::new("/"),
        deadline,
        phase,
    }
    .run()
    .map(|_| ())
}

/// Waits, bounded, for a recorded process identifier to disappear from `/proc`.
#[cfg(target_os = "linux")]
fn process_is_gone(pid_file: &Path) -> bool {
    let text = fs::read_to_string(pid_file).expect("the fixture recorded its descendant");
    let pid: u32 = text.trim().parse().expect("a numeric process identifier");
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Pipe descriptors this process holds, or `None` where procfs is unavailable.
///
/// Only tool invocations create pipes in this test binary, and the spawning tests are
/// serialized, so any growth is a descriptor one invocation failed to release.
fn open_pipes() -> Option<usize> {
    let entries = fs::read_dir("/proc/self/fd").ok()?;
    Some(
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                fs::read_link(entry.path())
                    .is_ok_and(|target| target.to_string_lossy().starts_with("pipe:"))
            })
            .count(),
    )
}

#[test]
fn descendants_that_hold_both_pipes_cannot_outlive_the_bounded_invocation() {
    let pid_file = scratch("descendant.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "/bin/sleep 300 & echo $! > {}; exit 0",
        pid_file.to_string_lossy()
    );
    let _serialized = serialized();
    let before = open_pipes();
    let started = Instant::now();

    let error = shell(&script, CompilePhase::FormatRoot, DEADLINE).expect_err("stray descendant");

    let elapsed = started.elapsed();
    assert_eq!(error.phase(), CompilePhase::FormatRoot);
    assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    assert!(
        elapsed <= DEADLINE + TERMINATION_GRACE,
        "the invocation took {elapsed:?}, beyond the deadline plus the declared grace"
    );
    #[cfg(target_os = "linux")]
    assert!(
        process_is_gone(&pid_file),
        "a descendant survived a failed build"
    );
    assert_eq!(
        open_pipes(),
        before,
        "the invocation leaked a pipe descriptor"
    );
}

#[test]
fn a_tool_that_ignores_the_polite_signal_is_forced_after_the_grace() {
    let pid_file = scratch("stubborn.pid");
    let _ = fs::remove_file(&pid_file);
    let script = format!(
        "trap '' TERM; /bin/sleep 300 & echo $! > {}; while : ; do /bin/sleep 1; done",
        pid_file.to_string_lossy()
    );
    let _serialized = serialized();
    let before = open_pipes();
    let started = Instant::now();

    let error = shell(&script, CompilePhase::BuildOverlay, DEADLINE).expect_err("stubborn tool");

    let elapsed = started.elapsed();
    assert_eq!(error.phase(), CompilePhase::BuildOverlay);
    assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    assert!(
        elapsed <= DEADLINE + TERMINATION_GRACE,
        "the invocation took {elapsed:?}, beyond the deadline plus the declared grace"
    );
    #[cfg(target_os = "linux")]
    assert!(
        process_is_gone(&pid_file),
        "a group member survived the force signal"
    );
    assert_eq!(
        open_pipes(),
        before,
        "the invocation leaked a pipe descriptor"
    );
}

#[test]
fn every_phase_reports_its_own_deadline_failure() {
    let _serialized = serialized();
    for phase in [
        CompilePhase::FormatRoot,
        CompilePhase::VerifyRoot,
        CompilePhase::BuildOverlay,
        CompilePhase::VerifyOverlay,
        CompilePhase::VerifyKernel,
        CompilePhase::StreamTree,
    ] {
        let error = shell("/bin/sleep 300", phase, DEADLINE).expect_err("deadline");
        assert_eq!(
            error.phase(),
            phase,
            "a failure was reported as another phase"
        );
        assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    }
}

#[test]
fn a_version_probe_failure_keeps_the_phase_that_asked_for_it() {
    let _serialized = serialized();
    let missing = scratch("no-such-tool");
    let _ = fs::remove_file(&missing);
    for phase in [CompilePhase::FormatRoot, CompilePhase::BuildOverlay] {
        let error = PinnedTool::open(&missing, phase).expect_err("a missing tool cannot be pinned");
        assert_eq!(error.phase(), phase);
        assert_eq!(error.kind(), CompileErrorKind::Toolchain);
        let error = super::version_line(
            &pinned(SHELL, phase),
            "--no-such-flag",
            Path::new("/"),
            phase,
        )
        .expect_err("failing version probe");
        assert_eq!(error.phase(), phase);
        assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    }
}

#[test]
fn a_feed_failure_terminates_the_tool_and_returns_the_feed_error() {
    let _serialized = serialized();
    let started = Instant::now();
    let before = open_pipes();
    let error = Invocation {
        program: &pinned(SHELL, CompilePhase::FormatRoot),
        arguments: vec![OsString::from("-c"), OsString::from("exec /bin/sleep 300")],
        environment: Vec::new(),
        working_directory: Path::new("/"),
        deadline: Duration::from_secs(120),
        phase: CompilePhase::FormatRoot,
    }
    .run_with_stdin(|_| {
        Err(CompileError::new(
            CompilePhase::StreamTree,
            CompileErrorKind::Integrity,
        ))
    })
    .expect_err("feed failure");

    assert_eq!(error.phase(), CompilePhase::StreamTree);
    assert_eq!(error.kind(), CompileErrorKind::Integrity);
    assert!(
        started.elapsed() <= TERMINATION_GRACE + Duration::from_secs(1),
        "a feed failure must not wait for the tool deadline"
    );
    assert_eq!(
        open_pipes(),
        before,
        "the invocation leaked a pipe descriptor"
    );
}

#[test]
fn a_bounded_tool_still_reports_its_output_and_exit_code() {
    let _serialized = serialized();
    let outcome = Invocation {
        program: &pinned(SHELL, CompilePhase::FormatRoot),
        arguments: vec![
            OsString::from("-c"),
            OsString::from("echo out; echo err >&2; exit 7"),
        ],
        environment: Vec::new(),
        working_directory: Path::new("/"),
        deadline: Duration::from_secs(30),
        phase: CompilePhase::FormatRoot,
    }
    .run()
    .expect("a bounded tool");

    assert_eq!(outcome.exit_code, Some(7));
    assert_eq!(outcome.stdout, b"out\n");
    assert_eq!(outcome.stderr, b"err\n");
    assert_eq!(outcome.program, "sh");
    assert!(!outcome.succeeded());
}

#[test]
fn the_declared_grace_is_a_containment_ceiling_not_a_latency_target() {
    assert!(TERMINATION_GRACE >= Duration::from_secs(6));
    assert!(TERMINATION_GRACE <= Duration::from_secs(30));
}
