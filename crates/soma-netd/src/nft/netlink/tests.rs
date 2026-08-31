//! Encoding and reply-parsing gates for the `nf_tables` read seam.
//!
//! The kernel is not needed to prove either direction: a request is a byte layout the tests
//! can assert in full, and a reply is a byte layout the tests can synthesize, including the
//! malformed ones a probe must refuse rather than read as an absence.

use super::*;

/// Builds one reply message with the given kind and body, padded as the kernel pads.
fn framed(kind: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let length = u32::try_from(NLMSG_HDRLEN + body.len()).expect("a bounded fixture");
    bytes.extend_from_slice(&length.to_ne_bytes());
    bytes.extend_from_slice(&kind.to_ne_bytes());
    bytes.extend_from_slice(&0_u16.to_ne_bytes());
    bytes.extend_from_slice(&1_u32.to_ne_bytes());
    bytes.extend_from_slice(&0_u32.to_ne_bytes());
    bytes.extend_from_slice(body);
    bytes
}

/// Builds one `NFT_MSG_NEWTABLE` body naming `name`.
fn table(name: &str) -> Vec<u8> {
    let mut body = vec![NFPROTO_INET, NFNETLINK_V0, 0, 0];
    let mut payload = name.as_bytes().to_vec();
    payload.push(0);
    let length = u16::try_from(4 + payload.len()).expect("a bounded fixture");
    body.extend_from_slice(&length.to_ne_bytes());
    body.extend_from_slice(&NFTA_TABLE_NAME.to_ne_bytes());
    body.extend_from_slice(&payload);
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    body
}

fn error(code: i32) -> Vec<u8> {
    (-code).to_ne_bytes().to_vec()
}

#[test]
fn a_named_request_carries_the_table_name_and_is_aligned_and_self_describing() {
    let bytes = request(NLM_F_REQUEST, Some("soma_0a1b2c3d"));

    assert!(bytes.len().is_multiple_of(4));
    assert_eq!(
        u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
        bytes.len()
    );
    assert_eq!(
        u16::from_ne_bytes([bytes[4], bytes[5]]),
        (NFNL_SUBSYS_NFTABLES << 8) | NFT_MSG_GETTABLE
    );
    assert_eq!(u16::from_ne_bytes([bytes[6], bytes[7]]), NLM_F_REQUEST);
    assert_eq!(bytes[NLMSG_HDRLEN], NFPROTO_INET);
    assert_eq!(bytes[NLMSG_HDRLEN + 1], NFNETLINK_V0);
    let attribute = &bytes[NLMSG_HDRLEN + NFGENMSG_LEN..];
    assert_eq!(
        u16::from_ne_bytes([attribute[2], attribute[3]]),
        NFTA_TABLE_NAME
    );
    assert_eq!(&attribute[4..attribute.len()], b"soma_0a1b2c3d\0\0\0");
}

#[test]
fn a_listing_request_names_no_table_and_asks_for_a_dump() {
    let bytes = request(NLM_F_REQUEST | NLM_F_DUMP, None);

    assert_eq!(bytes.len(), NLMSG_HDRLEN + NFGENMSG_LEN);
    assert_eq!(
        u16::from_ne_bytes([bytes[6], bytes[7]]),
        NLM_F_REQUEST | NLM_F_DUMP
    );
    assert_eq!(bytes[NLMSG_HDRLEN], NFPROTO_INET);
}

#[test]
fn presence_is_the_table_itself_and_absence_is_exactly_enoent() {
    let present = framed(kind_of(NFT_MSG_NEWTABLE), &table("soma_0a1b2c3d"));
    assert_eq!(presence(&present), Ok(true));

    let absent = framed(NLMSG_ERROR, &error(libc::ENOENT));
    assert_eq!(presence(&absent), Ok(false));

    let refused = framed(NLMSG_ERROR, &error(libc::EPERM));
    assert_eq!(
        presence(&refused),
        Err(Error::Kernel {
            step: Step::Netlink,
            errno: libc::EPERM
        }),
        "a refused question must not read as an absent table"
    );

    let acknowledged = framed(NLMSG_ERROR, &error(0));
    assert_eq!(presence(&acknowledged), Ok(true));
}

#[test]
fn a_malformed_reply_is_refused_rather_than_read_as_an_answer() {
    assert_eq!(
        presence(&[0; 8]),
        Err(Error::Protocol("netlink reply short"))
    );

    let mut truncated = framed(kind_of(NFT_MSG_NEWTABLE), &table("soma_0a1b2c3d"));
    truncated[..4].copy_from_slice(&999_u32.to_ne_bytes());
    assert_eq!(
        presence(&truncated),
        Err(Error::Protocol("netlink reply length"))
    );

    let foreign = framed(7, &table("soma_0a1b2c3d"));
    assert_eq!(
        presence(&foreign),
        Err(Error::Protocol("nftables reply kind"))
    );

    let short_error = framed(NLMSG_ERROR, &[0, 0]);
    assert_eq!(
        presence(&short_error),
        Err(Error::Protocol("netlink error short"))
    );
}

#[test]
fn a_dump_yields_every_table_name_and_ends_only_on_the_done_message() {
    let mut buffer = framed(kind_of(NFT_MSG_NEWTABLE), &table("soma_0a1b2c3d"));
    buffer.extend_from_slice(&framed(kind_of(NFT_MSG_NEWTABLE), &table("somah_0a1b2c3d")));
    let mut names = Vec::new();
    assert_eq!(collect(&buffer, &mut names), Ok(false));
    assert_eq!(names, vec!["soma_0a1b2c3d", "somah_0a1b2c3d"]);

    let mut tail = framed(kind_of(NFT_MSG_NEWTABLE), &table("soma_ffffffff"));
    tail.extend_from_slice(&framed(NLMSG_DONE, &[0; 4]));
    assert_eq!(collect(&tail, &mut names), Ok(true));
    assert_eq!(names.len(), 3);
}

#[test]
fn a_dump_that_fails_partway_is_an_error_rather_than_a_short_listing() {
    let mut buffer = framed(kind_of(NFT_MSG_NEWTABLE), &table("soma_0a1b2c3d"));
    buffer.extend_from_slice(&framed(NLMSG_ERROR, &error(libc::EPERM)));
    let mut names = Vec::new();

    assert_eq!(
        collect(&buffer, &mut names),
        Err(Error::Kernel {
            step: Step::Netlink,
            errno: libc::EPERM
        })
    );
}

#[test]
fn a_body_without_a_name_attribute_contributes_no_table() {
    let mut body = vec![NFPROTO_INET, NFNETLINK_V0, 0, 0];
    body.extend_from_slice(&8_u16.to_ne_bytes());
    body.extend_from_slice(&9_u16.to_ne_bytes());
    body.extend_from_slice(&[1, 2, 3, 4]);
    assert_eq!(table_name(&body), None);

    let mut names = Vec::new();
    let buffer = framed(kind_of(NFT_MSG_NEWTABLE), &body);
    assert_eq!(collect(&buffer, &mut names), Ok(false));
    assert!(names.is_empty());
}

#[test]
fn the_listing_of_a_live_namespace_is_at_worst_a_typed_failure() {
    // Without `CAP_NET_ADMIN` the kernel refuses the dump; the point of the call here is that
    // neither outcome panics and that a refusal is typed rather than an empty listing.
    match list_tables() {
        Ok(names) => assert!(names.iter().all(|name| !name.is_empty())),
        Err(error) => assert!(matches!(error, Error::Kernel { .. } | Error::Protocol(_))),
    }
}
