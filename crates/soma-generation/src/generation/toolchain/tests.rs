//! Canonical-serialization and binding-conflict tests for the sealed builder environment.

use super::*;

const PHASE: CompilePhase = CompilePhase::FormatRoot;
const FORMATTER: &str = "mkfs.erofs";
const CHECKER: &str = "fsck.erofs";

fn digest(byte: u8) -> Sha256Digest {
    Sha256Digest::from_bytes([byte; 32])
}

fn tool(name: &str, byte: u8, revision: &str) -> BoundTool {
    BoundTool::new(name, digest(byte), revision, PHASE).expect("a valid bound tool")
}

fn sealed(tools: &[BoundTool]) -> Sha256Digest {
    let mut environment = BuilderEnvironment::new();
    for bound in tools {
        environment.bind(bound.clone(), PHASE).expect("binds");
    }
    environment.digest(PHASE).expect("a sealed digest")
}

#[test]
fn the_environment_digest_does_not_depend_on_binding_order() {
    let first = tool(FORMATTER, 1, "1.9.4");
    let second = tool(CHECKER, 2, "1.9.4");
    let third = tool("mke2fs", 3, "1.47.0");

    let forward = sealed(&[first.clone(), second.clone(), third.clone()]);
    let backward = sealed(&[third, second, first]);

    assert_eq!(forward, backward);
}

#[test]
fn changing_any_bound_field_changes_the_environment_digest() {
    let base = sealed(&[tool(FORMATTER, 1, "1.9.4"), tool("mke2fs", 3, "1.47.0")]);

    assert_ne!(base, sealed(&[tool(FORMATTER, 1, "1.9.4")]));
    assert_ne!(
        base,
        sealed(&[tool(FORMATTER, 9, "1.9.4"), tool("mke2fs", 3, "1.47.0")])
    );
    assert_ne!(
        base,
        sealed(&[tool(FORMATTER, 1, "1.9.5"), tool("mke2fs", 3, "1.47.0")])
    );
    assert_ne!(
        base,
        sealed(&[tool(CHECKER, 1, "1.9.4"), tool("mke2fs", 3, "1.47.0")])
    );
    assert_ne!(
        base,
        sealed(&[
            tool(FORMATTER, 1, "1.9.4"),
            tool("mke2fs", 3, "1.47.0"),
            tool("debugfs", 4, "1.47.0"),
        ])
    );
}

#[test]
fn a_name_and_a_revision_cannot_be_confused_across_the_length_prefixes() {
    let split = sealed(&[tool("ab", 1, "cd")]);
    let joined = sealed(&[tool("abc", 1, "d")]);

    assert_ne!(split, joined);
}

#[test]
fn binding_one_name_to_different_bytes_is_an_integrity_failure() {
    let mut environment = BuilderEnvironment::new();
    environment
        .bind(tool("mke2fs", 1, "1.47.0"), PHASE)
        .expect("first bind");

    assert_eq!(
        environment
            .bind(tool("mke2fs", 1, "1.47.0"), PHASE)
            .map_err(|error| error.kind()),
        Ok(())
    );
    assert_eq!(
        environment
            .bind(tool("mke2fs", 2, "1.47.0"), PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::Integrity)
    );
    assert_eq!(
        environment
            .bind(tool("mke2fs", 1, "1.47.1"), PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::Integrity)
    );
    assert_eq!(environment.tools().len(), 1);
}

#[test]
fn an_environment_that_bound_no_tool_has_no_digest() {
    assert_eq!(
        BuilderEnvironment::new()
            .digest(PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::InvalidInput)
    );
}

#[test]
fn the_bound_tool_count_is_limited() {
    let mut environment = BuilderEnvironment::new();
    for index in 0..MAX_BOUND_TOOLS {
        environment
            .bind(tool(&format!("tool-{index:02}"), 1, ""), PHASE)
            .expect("within the limit");
    }

    assert_eq!(environment.tools().len(), MAX_BOUND_TOOLS);
    assert_eq!(
        environment
            .bind(tool("one-too-many", 1, ""), PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::LimitExceeded)
    );
}

#[test]
fn a_bound_tool_name_is_a_bare_printable_file_name() {
    for name in ["", "/usr/bin/mke2fs", "a/b", ".", "..", "mke\u{7}2fs"] {
        assert_eq!(
            BoundTool::new(name, digest(1), "1.47.0", PHASE).map_err(|error| error.kind()),
            Err(CompileErrorKind::InvalidInput),
            "{name:?} was accepted"
        );
    }
    assert_eq!(
        BoundTool::new(&"a".repeat(MAX_TOOL_FIELD_BYTES + 1), digest(1), "", PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::InvalidInput)
    );
    assert_eq!(
        BoundTool::new(
            "mke2fs",
            digest(1),
            &"v".repeat(MAX_TOOL_FIELD_BYTES + 1),
            PHASE
        )
        .map_err(|error| error.kind()),
        Err(CompileErrorKind::InvalidInput)
    );
    assert_eq!(
        BoundTool::new("mke2fs", digest(1), "1.47.0\n", PHASE).map_err(|error| error.kind()),
        Err(CompileErrorKind::InvalidInput)
    );
    let bound = BoundTool::new("mke2fs", digest(1), "1.47.0", PHASE).expect("valid");
    assert_eq!(bound.name(), "mke2fs");
    assert_eq!(bound.revision(), "1.47.0");
    assert_eq!(bound.digest(), digest(1));
}

#[test]
fn absorbing_another_environment_binds_every_tool_it_holds() {
    let mut root = BuilderEnvironment::new();
    root.bind(tool(FORMATTER, 1, "1.9.4"), PHASE)
        .expect("binds");
    let mut overlay = BuilderEnvironment::new();
    overlay
        .bind(tool("mke2fs", 3, "1.47.0"), PHASE)
        .expect("binds");
    overlay
        .bind(tool("debugfs", 4, "1.47.0"), PHASE)
        .expect("binds");

    let mut combined = root.clone();
    combined.absorb(&overlay, PHASE).expect("absorbs");

    assert_eq!(
        combined
            .tools()
            .iter()
            .map(BoundTool::name)
            .collect::<Vec<_>>(),
        ["debugfs", "mke2fs", FORMATTER]
    );
    assert_eq!(
        combined.digest(PHASE).expect("digest"),
        sealed(&[
            tool(FORMATTER, 1, "1.9.4"),
            tool("mke2fs", 3, "1.47.0"),
            tool("debugfs", 4, "1.47.0"),
        ])
    );
    let mut conflicting = combined.clone();
    assert_eq!(
        conflicting
            .absorb(&conflict(), PHASE)
            .map_err(|error| error.kind()),
        Err(CompileErrorKind::Integrity)
    );
}

fn conflict() -> BuilderEnvironment {
    let mut environment = BuilderEnvironment::new();
    environment
        .bind(tool("mke2fs", 9, "1.47.0"), PHASE)
        .expect("binds");
    environment
}
