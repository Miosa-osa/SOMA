use super::{MAX_HANDSHAKE_MESSAGE, unframe};

#[test]
fn every_handshake_u16_declaration_obeys_the_exact_length_rule() {
    let body = [0xA5; 32];
    for declared in 0_u16..=u16::MAX {
        let mut framed = Vec::with_capacity(2 + body.len());
        framed.extend_from_slice(&declared.to_be_bytes());
        framed.extend_from_slice(&body);
        assert_eq!(
            unframe(&framed).is_ok(),
            usize::from(declared) == body.len()
        );
    }
}

#[test]
fn handshake_boundary_matrix_enforces_nonempty_bounded_messages() {
    for body_length in 0..=MAX_HANDSHAKE_MESSAGE + 1 {
        let declared = u16::try_from(body_length).expect("small boundary matrix");
        let mut framed = Vec::with_capacity(2 + body_length);
        framed.extend_from_slice(&declared.to_be_bytes());
        framed.resize(2 + body_length, 0xA5);
        assert_eq!(
            unframe(&framed).is_ok(),
            (1..=MAX_HANDSHAKE_MESSAGE).contains(&body_length)
        );
    }
}
