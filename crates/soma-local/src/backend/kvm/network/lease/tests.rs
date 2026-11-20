use super::*;

/// The broker's reply is the launch page, so the two must carry the same values.
///
/// The reply carries the fields as fixed bytes rather than as a structure, so an offset that
/// drifted would hand the guest a plausible address belonging to no lease. Encoding what the
/// broker encodes and decoding it back is the only check that catches that.
#[test]
fn the_decoded_launch_values_are_the_ones_the_broker_encoded() {
    let expected = LaunchNetwork::new(
        9,
        3,
        [0x02, 1, 2, 3, 4, 5],
        [10, 200, 0, 6],
        30,
        [10, 200, 0, 5],
        [1, 1, 1, 1],
        0x0102_0304_0506_0708,
    )
    .expect("launch values");
    let mut bytes = [0_u8; 35];
    bytes[..4].copy_from_slice(&expected.vsock_cid().to_be_bytes());
    bytes[4..8].copy_from_slice(&expected.generation().to_be_bytes());
    bytes[8..14].copy_from_slice(&expected.mac());
    bytes[14..18].copy_from_slice(&expected.address());
    bytes[18] = expected.prefix_length();
    bytes[19..23].copy_from_slice(&expected.gateway());
    bytes[23..27].copy_from_slice(&expected.resolver());
    bytes[27..35].copy_from_slice(&expected.time_sample_nanos().to_be_bytes());

    let decoded = decode_launch(&bytes).expect("decodes");
    assert_eq!(decoded.vsock_cid(), expected.vsock_cid());
    assert_eq!(decoded.generation(), expected.generation());
    assert_eq!(decoded.mac(), expected.mac());
    assert_eq!(decoded.address(), expected.address());
    assert_eq!(decoded.prefix_length(), expected.prefix_length());
    assert_eq!(decoded.gateway(), expected.gateway());
    assert_eq!(decoded.resolver(), expected.resolver());
    assert_eq!(decoded.time_sample_nanos(), expected.time_sample_nanos());
}

/// A reply no broker could have produced must never become a launch page.
#[test]
fn launch_values_no_broker_could_have_leased_are_refused() {
    assert!(decode_launch(&[0_u8; 35]).is_none());
}
