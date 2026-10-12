use std::ffi::OsString;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::Path;

use super::{MAX_FILE_BYTES, MAX_VARIABLES, load_from, parse};

fn pairs(text: &[u8]) -> Vec<(String, String)> {
    parse(text)
        .into_iter()
        .map(|(name, value)| {
            (
                name.into_string().expect("name"),
                value.into_string().expect("value"),
            )
        })
        .collect()
}

/// The user running the test: whoever owns a file the test just created.
fn me() -> u32 {
    let file = tempfile::NamedTempFile::new().expect("temporary file");
    fs::metadata(file.path()).expect("metadata").uid()
}

fn write(directory: &Path, name: &str, text: &str, mode: u32) -> std::path::PathBuf {
    let path = directory.join(name);
    fs::write(&path, text).expect("write");
    fs::set_permissions(&path, fs::Permissions::from_mode(mode)).expect("mode");
    path
}

#[test]
fn quoted_bare_and_escaped_values_parse_to_what_a_shell_would_read() {
    let text = b"A='1'\nB='it'\"'\"'s'\nC=plain/path-1.2\nEMPTY=''\nSPACED='two words'\n";
    assert_eq!(
        pairs(text),
        vec![
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "it's".to_owned()),
            ("C".to_owned(), "plain/path-1.2".to_owned()),
            ("EMPTY".to_owned(), String::new()),
            ("SPACED".to_owned(), "two words".to_owned()),
        ]
    );
}

#[test]
fn a_value_that_holds_a_newline_or_shell_syntax_survives_inside_quotes() {
    assert_eq!(
        pairs(b"A='line1\nline2'\n").len(),
        0,
        "a quoted value cannot span lines here, so the line is dropped rather than half read"
    );
    assert_eq!(
        pairs(b"A='$(id) `x` ; | &'\n"),
        vec![("A".to_owned(), "$(id) `x` ; | &".to_owned())]
    );
}

#[test]
fn malformed_lines_are_skipped_and_never_become_another_variable() {
    let text = b"# comment\n\nNOEQUALS\n1BAD='x'\nA B='x'\nUNCLOSED='abc\nBARE=has space\nBARE2=a;b\nTRAIL='a'b\nOK='1'\nNUL='a\0b'\n=x\n";
    assert_eq!(pairs(text), vec![("OK".to_owned(), "1".to_owned())]);
}

#[test]
fn a_later_line_replaces_an_earlier_one_in_place() {
    assert_eq!(
        pairs(b"A='1'\nB='2'\nA='3'\n"),
        vec![
            ("A".to_owned(), "3".to_owned()),
            ("B".to_owned(), "2".to_owned())
        ]
    );
}

#[test]
fn crlf_line_endings_are_read_as_lf() {
    assert_eq!(pairs(b"A='1'\r\n"), vec![("A".to_owned(), "1".to_owned())]);
}

#[test]
fn the_variable_count_and_value_length_are_bounded() {
    let many = (0..MAX_VARIABLES + 10)
        .map(|index| format!("V{index}='x'\n"))
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(pairs(many.as_bytes()).len(), MAX_VARIABLES);
    let long = format!("A='{}'\n", "x".repeat(4097));
    assert!(pairs(long.as_bytes()).is_empty());
    let at_bound = format!("A='{}'\n", "x".repeat(4096));
    assert_eq!(pairs(at_bound.as_bytes()).len(), 1);
}

#[test]
fn a_file_the_agent_could_have_trusted_is_read() {
    let directory = tempfile::tempdir().expect("directory");
    let path = write(directory.path(), "identity.env", "A='1'\n", 0o600);
    assert_eq!(
        load_from(&path, me()),
        vec![(OsString::from("A"), OsString::from("1"))]
    );
    let group_readable = write(directory.path(), "readable.env", "A='1'\n", 0o644);
    assert_eq!(load_from(&group_readable, me()).len(), 1);
}

#[test]
fn a_file_anyone_else_could_have_written_is_ignored_whole() {
    let directory = tempfile::tempdir().expect("directory");
    for mode in [0o666, 0o662, 0o620, 0o602] {
        let path = write(directory.path(), "writable.env", "A='1'\n", mode);
        assert!(load_from(&path, me()).is_empty(), "mode {mode:o}");
    }
}

#[test]
fn a_file_owned_by_someone_else_is_ignored() {
    let directory = tempfile::tempdir().expect("directory");
    let path = write(directory.path(), "identity.env", "A='1'\n", 0o600);
    assert!(load_from(&path, me() + 1).is_empty());
}

#[test]
fn a_link_a_directory_a_missing_file_and_an_oversized_file_are_ignored() {
    let directory = tempfile::tempdir().expect("directory");
    let real = write(directory.path(), "real.env", "A='1'\n", 0o600);
    let link = directory.path().join("link.env");
    symlink(&real, &link).expect("symlink");
    assert!(load_from(&link, me()).is_empty());
    assert!(load_from(directory.path(), me()).is_empty());
    assert!(load_from(&directory.path().join("missing.env"), me()).is_empty());
    let big = "A='x'\n".repeat(usize::try_from(MAX_FILE_BYTES).expect("bound") / 6 + 10);
    let oversized = write(directory.path(), "big.env", &big, 0o600);
    assert!(load_from(&oversized, me()).is_empty());
}
