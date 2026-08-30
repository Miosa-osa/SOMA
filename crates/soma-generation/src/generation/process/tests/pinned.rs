//! Binding one tool to the exact bytes it will execute.

use std::fs;
use std::path::Path;
use std::time::Duration;

use super::super::super::artifacts::Sha256Digest;
use super::super::{CompileErrorKind, CompilePhase, Invocation, PinnedTool};
use super::{scratch, serialized};

#[test]
fn a_pinned_tool_runs_the_bytes_it_measured_after_its_path_is_replaced() {
    let _serialized = serialized();
    let phase = CompilePhase::FormatRoot;
    let program = scratch("replaceable-tool");
    let _ = fs::remove_file(&program);
    write_script(&program, "#!/bin/sh\necho measured\n");

    let tool = PinnedTool::open(&program, phase).expect("a pinned tool");
    let measured = tool.digest();

    // The path now names entirely different bytes, which is the window the old hash-then-spawn
    // sequence left open.
    fs::remove_file(&program).expect("the tool path is replaceable");
    write_script(&program, "#!/bin/sh\necho substituted\n");
    let replaced = PinnedTool::open(&program, phase).expect("the replacement");

    let outcome = Invocation {
        program: &tool,
        arguments: Vec::new(),
        environment: Vec::new(),
        working_directory: Path::new("/"),
        deadline: Duration::from_secs(30),
        phase,
    }
    .run()
    .expect("the measured tool runs");

    assert_ne!(measured, replaced.digest());
    assert_eq!(outcome.stdout, b"measured\n");
    assert_eq!(outcome.exit_code, Some(0));
    assert_eq!(outcome.program, "replaceable-tool");
    let _ = fs::remove_file(&program);
}

#[test]
fn a_pinned_tool_measures_the_descriptor_it_will_execute() {
    let phase = CompilePhase::BuildOverlay;
    let program = scratch("measured-tool");
    let _ = fs::remove_file(&program);
    write_script(&program, "#!/bin/sh\nexit 0\n");
    let bytes = fs::read(&program).expect("the fixture bytes");

    let tool = PinnedTool::open(&program, phase).expect("a pinned tool");

    assert_eq!(tool.digest(), Sha256Digest::of(&bytes));
    assert_eq!(tool.name(), "measured-tool");
    assert!(tool.require_bound(phase).is_ok());
    assert!(
        tool.program() != program,
        "a pinned tool must execute its descriptor rather than its original path"
    );
    let _ = fs::remove_file(&program);
}

#[test]
fn a_directory_and_an_empty_file_are_not_pinnable_tools() {
    let phase = CompilePhase::FormatRoot;
    let directory = scratch("");
    let empty = scratch("empty-tool");
    let _ = fs::remove_file(&empty);
    fs::write(&empty, b"").expect("an empty file");

    for path in [directory.as_path(), empty.as_path()] {
        let error = PinnedTool::open(path, phase).expect_err("not a runnable tool");
        assert_eq!(error.phase(), phase);
        assert_eq!(error.kind(), CompileErrorKind::Toolchain);
    }
    let _ = fs::remove_file(&empty);
}

fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt as _;

    fs::write(path, body).expect("the fixture script");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("an executable fixture");
}
