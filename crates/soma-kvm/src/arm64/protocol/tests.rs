use super::*;

fn sample(payload: Vec<u8>) -> Frame {
    Frame {
        kind: Kind::Stdout,
        request_id: 7,
        sequence: 2,
        challenge: [9; 32],
        payload,
    }
}

fn decode(bytes: &[u8]) -> Result<Option<Frame>, &'static str> {
    let mut decoder = Decoder::new();
    let mut received = None;
    for &byte in bytes {
        received = decoder.push(byte)?.or(received);
    }
    Ok(received)
}

fn assert_poisoned(bytes: Vec<u8>) {
    let mut decoder = Decoder::new();
    assert!(bytes.into_iter().any(|byte| decoder.push(byte).is_err()));
    assert_eq!(decoder.push(0), Err("frame decoder is poisoned"));
}

#[test]
fn crc32c_matches_the_standard_check_value() {
    assert_eq!(crc32c(b"123456789", b""), 0xe306_9283);
}

#[test]
fn frame_round_trip_is_incremental_and_exact() {
    let frame = sample(b"exact bytes".to_vec());
    assert_eq!(decode(&encode(&frame).unwrap()).unwrap(), Some(frame));
}

#[test]
fn decoder_accepts_two_consecutive_frames() {
    let first = sample(b"first".to_vec());
    let mut second = sample(b"second".to_vec());
    second.sequence = 3;
    let mut decoder = Decoder::new();
    let mut received = Vec::new();
    for byte in encode(&first)
        .unwrap()
        .into_iter()
        .chain(encode(&second).unwrap())
    {
        if let Some(frame) = decoder.push(byte).unwrap() {
            received.push(frame);
        }
    }
    assert_eq!(received, vec![first, second]);
}

#[test]
fn incomplete_frames_never_emit() {
    let encoded = encode(&sample(vec![1, 2, 3])).unwrap();
    assert_eq!(decode(&encoded[..HEADER_LEN - 1]).unwrap(), None);
    assert_eq!(decode(&encoded[..encoded.len() - 1]).unwrap(), None);
}

#[test]
fn exact_maximum_payload_round_trips_and_one_over_is_rejected() {
    let maximum = sample(vec![0xa5; MAX_PAYLOAD]);
    assert_eq!(decode(&encode(&maximum).unwrap()).unwrap(), Some(maximum));
    assert_eq!(
        encode(&sample(vec![0; MAX_PAYLOAD + 1])),
        Err("payload exceeds protocol limit")
    );
}

#[test]
fn decoder_rejects_every_header_identity_mutation_and_stays_poisoned() {
    let valid = encode(&sample(vec![7])).unwrap();
    for (offset, value) in [
        (0, b'X'),
        (4, VERSION + 1),
        (5, u8::try_from(HEADER_LEN - 1).unwrap()),
        (6, u8::MAX),
        (7, 1),
        (11, 1),
    ] {
        let mut invalid = valid.clone();
        invalid[offset] = value;
        assert_poisoned(invalid);
    }
}

#[test]
fn decoder_rejects_bad_crc_and_oversized_declared_payload() {
    let valid = encode(&sample(vec![7])).unwrap();
    let mut bad_crc = valid.clone();
    *bad_crc.last_mut().unwrap() ^= 1;
    assert_poisoned(bad_crc);

    let mut oversized = valid;
    oversized[24..28].copy_from_slice(&(u32::try_from(MAX_PAYLOAD).unwrap() + 1).to_be_bytes());
    assert_poisoned(oversized);
}

#[test]
fn frame_debug_redacts_challenge_and_payload() {
    let frame = Frame {
        kind: Kind::Request,
        request_id: 4,
        sequence: 0,
        challenge: [0xaa; 32],
        payload: b"secret".to_vec(),
    };
    assert_eq!(
        format!("{frame:?}"),
        "Frame { kind: Request, request_id: 4, sequence: 0, payload_len: 6, .. }"
    );
}
