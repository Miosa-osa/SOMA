use super::{Node, TreeBounds, TreeDecoder, bounds, decode, entry, good, invalid, limit, manifest};
use crate::generation::error::{CompileErrorKind, CompilePhase};

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
