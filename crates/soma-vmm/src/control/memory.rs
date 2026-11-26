//! The memory device's answer as one reply word: its level, or why it refused.

use crate::sandbox::{MemoryError, MemoryLevel};

use super::{ControlError, field};

/// `memory requested=<bytes> plugged=<bytes> region=<bytes>` or `memory_refused <reason>`.
pub(super) fn encode(answer: &Result<MemoryLevel, MemoryError>) -> String {
    match answer {
        Ok(level) => format!(
            "memory requested={} plugged={} region={}",
            level.requested_bytes, level.plugged_bytes, level.region_bytes
        ),
        Err(error) => format!("memory_refused {}", reason(*error)),
    }
}

const fn reason(error: MemoryError) -> &'static str {
    match error {
        MemoryError::NoDevice => "no_device",
        MemoryError::BadSize => "bad_size",
        MemoryError::Busy => "busy",
    }
}

/// Reads the level a `memory` reply carries.
pub(super) fn level(text: &str) -> Result<MemoryLevel, ControlError> {
    let mut tokens = text.split_whitespace();
    let mut value = |name: &'static str| {
        tokens
            .next()
            .and_then(|token| token.strip_prefix(name)?.strip_prefix('='))
            .and_then(|digits| digits.parse::<u64>().ok())
            .ok_or(ControlError::InvalidValue(name))
    };
    let level = MemoryLevel {
        requested_bytes: value("requested")?,
        plugged_bytes: value("plugged")?,
        region_bytes: value("region")?,
    };
    field::end(tokens)?;
    Ok(level)
}

/// Reads the refusal a `memory_refused` reply carries.
pub(super) fn refusal(text: &str) -> Result<MemoryError, ControlError> {
    let mut tokens = text.split_whitespace();
    let error = match tokens.next() {
        Some("no_device") => MemoryError::NoDevice,
        Some("bad_size") => MemoryError::BadSize,
        Some("busy") => MemoryError::Busy,
        _ => return Err(ControlError::InvalidValue("memory refusal")),
    };
    field::end(tokens)?;
    Ok(error)
}

#[cfg(test)]
mod tests {
    use super::{MemoryError, MemoryLevel, encode, level, refusal};

    #[test]
    fn a_memory_answer_reads_back_as_it_was_sent() {
        let sent = MemoryLevel {
            requested_bytes: 512 << 20,
            plugged_bytes: 256 << 20,
            region_bytes: 15 << 30,
        };
        let text = encode(&Ok(sent));
        let (head, rest) = text.split_once(' ').expect("a word and fields");
        assert_eq!(head, "memory");
        assert_eq!(level(rest), Ok(sent));
        for error in [
            MemoryError::NoDevice,
            MemoryError::BadSize,
            MemoryError::Busy,
        ] {
            let text = encode(&Err(error));
            let (head, rest) = text.split_once(' ').expect("a word and a reason");
            assert_eq!(head, "memory_refused");
            assert_eq!(refusal(rest), Ok(error));
        }
        assert!(level("requested=1 plugged=2").is_err());
        assert!(level("plugged=1 requested=2 region=3").is_err());
        assert!(level("requested=1 plugged=2 region=3 extra").is_err());
        assert!(refusal("tired").is_err());
    }
}
