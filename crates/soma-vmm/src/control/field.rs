//! Field encoding for control packets.
//!
//! Identifiers and command bytes are hexadecimal so that a packet is always one line of ASCII
//! with no separator ambiguity, whatever bytes a program path or an argument holds.

use std::str::FromStr;

use super::ControlError;

/// Lowercase hexadecimal for one byte string.
pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: [u8; 16] = *b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

/// Decodes one hexadecimal field into its bytes.
pub(super) fn bytes(token: Option<&str>, field: &'static str) -> Result<Vec<u8>, ControlError> {
    let text = token.ok_or(ControlError::MissingField(field))?;
    if text.len() % 2 != 0 {
        return Err(ControlError::InvalidValue(field));
    }
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for index in (0..text.len()).step_by(2) {
        let pair = text
            .get(index..index + 2)
            .ok_or(ControlError::InvalidValue(field))?;
        bytes.push(u8::from_str_radix(pair, 16).map_err(|_| ControlError::InvalidValue(field))?);
    }
    Ok(bytes)
}

/// Decodes one hexadecimal field of exactly `WIDTH` bytes, the width every identifier has.
pub(super) fn identifier<const WIDTH: usize>(
    token: Option<&str>,
    field: &'static str,
) -> Result<[u8; WIDTH], ControlError> {
    bytes(token, field)?
        .try_into()
        .map_err(|_| ControlError::InvalidValue(field))
}

/// Decodes one decimal field.
pub(super) fn number<T: FromStr>(
    token: Option<&str>,
    field: &'static str,
) -> Result<T, ControlError> {
    token
        .ok_or(ControlError::MissingField(field))?
        .parse()
        .map_err(|_| ControlError::InvalidValue(field))
}

/// Rejects a packet that carries more fields than its form takes.
pub(super) fn end<'a>(mut tokens: impl Iterator<Item = &'a str>) -> Result<(), ControlError> {
    if tokens.next().is_some() {
        Err(ControlError::TrailingField)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_fields_round_trip() {
        let original = [0u8, 1, 15, 16, 254, 255];
        assert_eq!(hex(&original), "00010f10feff");
        assert_eq!(bytes(Some("00010f10feff"), "field"), Ok(original.to_vec()));
        assert_eq!(identifier::<6>(Some("00010f10feff"), "field"), Ok(original));
    }

    #[test]
    fn malformed_fields_are_rejected_by_name() {
        assert_eq!(
            bytes(None, "program"),
            Err(ControlError::MissingField("program"))
        );
        assert_eq!(
            bytes(Some("abc"), "program"),
            Err(ControlError::InvalidValue("program"))
        );
        assert_eq!(
            bytes(Some("zz"), "program"),
            Err(ControlError::InvalidValue("program"))
        );
        assert_eq!(
            identifier::<4>(Some("0102"), "operation"),
            Err(ControlError::InvalidValue("operation"))
        );
        assert_eq!(
            number::<u32>(Some("-1"), "timeout"),
            Err(ControlError::InvalidValue("timeout"))
        );
        assert_eq!(end(["extra"].into_iter()), Err(ControlError::TrailingField));
        assert_eq!(end(std::iter::empty()), Ok(()));
    }
}
