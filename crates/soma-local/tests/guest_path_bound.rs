//! The portable facade's path rule must be the guest protocol's path rule.
//!
//! `soma` sits below `soma-guest` and cannot call into it, so it restates the bound a surface
//! needs in order to refuse an inadmissible path before it reaches the wire. This crate depends
//! on both, which makes it the one place the two can be compared. If they ever disagree, a path
//! one accepts and the other rejects would arrive at the guest as a protocol fault and end the
//! session, which is exactly the outcome the facade's check exists to prevent.

/// Every path either both admit or both refuse.
#[test]
fn the_facade_and_the_protocol_admit_exactly_the_same_paths() {
    let cases: Vec<Vec<u8>> = vec![
        b"/".to_vec(),
        b"/workspace".to_vec(),
        b"/workspace/main.js".to_vec(),
        b"/workspace/\xff\xfe".to_vec(),
        b"relative/path".to_vec(),
        b"".to_vec(),
        b"/holds\0a/nul".to_vec(),
        vec![b'/'; soma::MAX_GUEST_PATH_BYTES],
        vec![b'/'; soma::MAX_GUEST_PATH_BYTES + 1],
    ];

    for path in cases {
        let facade = soma::check_guest_path(&path).is_ok();
        // The protocol's own check is reached by asking it to encode and decode the request:
        // that is the exact path a real request takes, so nothing here can drift from it.
        let protocol = soma_guest::FileRequest::decode_body(
            &soma_guest::FileRequest::Exists {
                path: path.as_slice().into(),
            }
            .encode_body(),
        )
        .is_ok();
        assert_eq!(
            facade,
            protocol,
            "the two disagree about a {}-byte path beginning {:?}",
            path.len(),
            &path[..path.len().min(8)]
        );
    }
}

/// The restated bound is the protocol's own bound, not merely compatible with it.
#[test]
fn the_restated_path_bound_is_the_protocol_bound() {
    assert_eq!(soma::MAX_GUEST_PATH_BYTES, soma_guest::MAX_PATH_BYTES);
}
