use super::{TreeBounds, TreeDecoder, TreeNode};
use crate::generation::error::{CompileErrorKind, CompilePhase};

const DIGEST: [u8; 32] = [0x5a; 32];

fn bounds() -> TreeBounds {
    TreeBounds {
        max_entries: 64,
        max_path_bytes: 64,
        max_link_bytes: 64,
        max_metadata_bytes: 4096,
        max_file_bytes: 1 << 20,
        max_content_bytes: 1 << 22,
    }
}

enum Node<'a> {
    Dir,
    File(u64),
    Link(&'a [u8]),
    Hard(&'a [u8]),
    Fifo,
    Kind(u8),
}

struct Entry<'a> {
    path: &'a [u8],
    mode: u32,
    node: Node<'a>,
    xattrs: u32,
}

fn entry<'a>(path: &'a [u8], mode: u32, node: Node<'a>) -> Entry<'a> {
    Entry {
        path,
        mode,
        node,
        xattrs: 0,
    }
}

fn manifest(count: u32, entries: &[Entry<'_>]) -> Vec<u8> {
    let mut bytes = b"SOMARFS\0".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&count.to_be_bytes());
    for item in entries {
        bytes.extend_from_slice(&(item.path.len() as u32).to_be_bytes());
        bytes.extend_from_slice(item.path);
        let kind = match item.node {
            Node::Dir => 1,
            Node::File(_) => 2,
            Node::Link(_) => 3,
            Node::Fifo => 4,
            Node::Hard(_) => 5,
            Node::Kind(kind) => kind,
        };
        bytes.push(kind);
        bytes.extend_from_slice(&item.mode.to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&7_u64.to_be_bytes());
        bytes.extend_from_slice(&item.xattrs.to_be_bytes());
        match item.node {
            Node::File(size) => {
                bytes.extend_from_slice(&size.to_be_bytes());
                bytes.extend_from_slice(&DIGEST);
            }
            Node::Link(value) | Node::Hard(value) => {
                bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
                bytes.extend_from_slice(value);
            }
            Node::Dir | Node::Fifo | Node::Kind(_) => {}
        }
    }
    bytes
}

fn decode(bytes: &[u8]) -> Result<Vec<TreeNode>, (CompilePhase, CompileErrorKind)> {
    let redact = |error: crate::CompileError| (error.phase(), error.kind());
    let mut decoder = TreeDecoder::new(bytes, bounds()).map_err(redact)?;
    let mut nodes = Vec::new();
    for item in decoder.by_ref() {
        nodes.push(item.map_err(redact)?.node);
    }
    decoder.finish().map_err(redact)?;
    Ok(nodes)
}

fn good() -> Vec<u8> {
    manifest(
        6,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o644, Node::File(3)),
            entry(b"b", 0o644, Node::Hard(b"a")),
            entry(b"d", 0o1777, Node::Dir),
            entry(b"d/l", 0o777, Node::Link(b"../a")),
            entry(b"p", 0o600, Node::Fifo),
        ],
    )
}

#[test]
fn decodes_every_supported_node_kind_in_order() {
    let nodes = decode(&good()).unwrap();
    assert_eq!(nodes.len(), 6);
    assert!(matches!(nodes[2], TreeNode::Hardlink { ref anchor } if anchor == b"a"));
    assert!(matches!(nodes[4], TreeNode::Symlink { ref target } if target == b"../a"));
}

fn invalid() -> (CompilePhase, CompileErrorKind) {
    (CompilePhase::DecodeTree, CompileErrorKind::InvalidInput)
}

fn limit() -> (CompilePhase, CompileErrorKind) {
    (CompilePhase::DecodeTree, CompileErrorKind::LimitExceeded)
}

#[test]
fn rejects_header_faults_truncation_and_trailing_bytes() {
    let mut magic = good();
    magic[0] = b'X';
    assert_eq!(decode(&magic).unwrap_err(), invalid());
    let mut version = good();
    version[9] = 2;
    assert_eq!(decode(&version).unwrap_err(), invalid());
    assert_eq!(decode(&manifest(0, &[])).unwrap_err(), invalid());
    assert_eq!(decode(&manifest(65, &[])).unwrap_err(), limit());
    let bytes = good();
    for length in 0..bytes.len() {
        assert!(
            decode(&bytes[..length]).is_err(),
            "prefix {length} accepted"
        );
    }
    let mut trailing = good();
    trailing.push(0);
    assert_eq!(decode(&trailing).unwrap_err(), invalid());
    let short_count = manifest(2, &[entry(b"", 0o755, Node::Dir)]);
    assert_eq!(decode(&short_count).unwrap_err(), invalid());
}

#[test]
fn rejects_root_and_ordering_violations() {
    let root_last = manifest(
        2,
        &[
            entry(b"a", 0o644, Node::File(1)),
            entry(b"", 0o755, Node::Dir),
        ],
    );
    assert_eq!(decode(&root_last).unwrap_err(), invalid());
    let root_file = manifest(1, &[entry(b"", 0o644, Node::File(1))]);
    assert_eq!(decode(&root_file).unwrap_err(), invalid());
    let unsorted = manifest(
        3,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"b", 0o644, Node::File(1)),
            entry(b"a", 0o644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&unsorted).unwrap_err(), invalid());
    let duplicate = manifest(
        3,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o644, Node::File(1)),
            entry(b"a", 0o644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&duplicate).unwrap_err(), invalid());
}

#[test]
fn rejects_unsafe_paths_and_missing_or_non_directory_parents() {
    for path in [
        b"/abs".as_slice(),
        b"a/",
        b"a//b",
        b"./a",
        b"a/../b",
        b"..",
        b"nul\0",
        b"missing/child",
    ] {
        let bytes = manifest(
            2,
            &[
                entry(b"", 0o755, Node::Dir),
                entry(path, 0o644, Node::File(1)),
            ],
        );
        assert_eq!(decode(&bytes).unwrap_err(), invalid(), "{path:?} accepted");
    }
    let file_parent = manifest(
        3,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"f", 0o644, Node::File(1)),
            entry(b"f/child", 0o644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&file_parent).unwrap_err(), invalid());
}

#[test]
fn rejects_metadata_and_link_violations() {
    let mode = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o10_644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&mode).unwrap_err(), invalid());
    let mut with_xattr = entry(b"a", 0o644, Node::File(1));
    with_xattr.xattrs = 1;
    let xattr = manifest(2, &[entry(b"", 0o755, Node::Dir), with_xattr]);
    assert_eq!(decode(&xattr).unwrap_err(), invalid());
    let kind = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o644, Node::Kind(9)),
        ],
    );
    assert_eq!(
        decode(&kind).unwrap_err(),
        (CompilePhase::DecodeTree, CompileErrorKind::Unsupported)
    );
    let dangling = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"b", 0o644, Node::Hard(b"a")),
        ],
    );
    assert_eq!(decode(&dangling).unwrap_err(), invalid());
    let forward = manifest(
        3,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o644, Node::Hard(b"b")),
            entry(b"b", 0o644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&forward).unwrap_err(), invalid());
    let empty_link = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"l", 0o777, Node::Link(b"")),
        ],
    );
    assert_eq!(decode(&empty_link).unwrap_err(), invalid());
    let nul_link = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"l", 0o777, Node::Link(b"a\0")),
        ],
    );
    assert_eq!(decode(&nul_link).unwrap_err(), invalid());
}

#[test]
fn enforces_every_configured_bound() {
    let long = [b'x'; 65];
    let path = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(&long, 0o644, Node::File(1)),
        ],
    );
    assert_eq!(decode(&path).unwrap_err(), limit());
    let link = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"l", 0o777, Node::Link(&long)),
        ],
    );
    assert_eq!(decode(&link).unwrap_err(), limit());
    let file = manifest(
        2,
        &[
            entry(b"", 0o755, Node::Dir),
            entry(b"a", 0o644, Node::File(1 << 21)),
        ],
    );
    assert_eq!(decode(&file).unwrap_err(), limit());
    let mut entries = vec![entry(b"", 0o755, Node::Dir)];
    let names: Vec<[u8; 1]> = (b'a'..=b'e').map(|byte| [byte]).collect();
    for name in &names {
        entries.push(entry(name, 0o644, Node::File(1 << 20)));
    }
    let aggregate = manifest(6, &entries);
    assert_eq!(decode(&aggregate).unwrap_err(), limit());
    let bytes = good();
    let mut wide = TreeDecoder::new(
        &bytes,
        TreeBounds {
            max_metadata_bytes: 8,
            ..bounds()
        },
    )
    .unwrap();
    let outcome: Result<Vec<_>, _> = wide.by_ref().collect();
    assert_eq!(outcome.unwrap_err().kind(), CompileErrorKind::LimitExceeded);
}
