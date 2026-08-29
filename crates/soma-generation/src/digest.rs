use std::io::Read;

use sha2::{Digest as _, Sha256};
use soma::OciDigest;

use crate::{ImportError, ImportErrorKind, ImportPhase};

pub(crate) fn parse(
    value: impl Into<String>,
    phase: ImportPhase,
) -> Result<OciDigest, ImportError> {
    OciDigest::parse(value).map_err(|_| ImportError::new(phase, ImportErrorKind::InvalidInput))
}

pub(crate) fn bytes(bytes: &[u8]) -> OciDigest {
    let output = Sha256::digest(bytes);
    from_output(output.as_ref())
}

pub(crate) fn reader(
    reader: &mut impl Read,
    maximum: u64,
    phase: ImportPhase,
) -> Result<(OciDigest, u64), ImportError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    let mut total = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| ImportError::new(phase, ImportErrorKind::Io))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(
                u64::try_from(count)
                    .map_err(|_| ImportError::new(phase, ImportErrorKind::LimitExceeded))?,
            )
            .ok_or_else(|| ImportError::new(phase, ImportErrorKind::LimitExceeded))?;
        if total > maximum {
            return Err(ImportError::new(phase, ImportErrorKind::LimitExceeded));
        }
        hasher.update(&buffer[..count]);
    }
    let output = hasher.finalize();
    Ok((from_output(output.as_ref()), total))
}

pub(crate) fn hex(digest: &OciDigest) -> &str {
    digest
        .as_str()
        .strip_prefix("sha256:")
        .expect("OciDigest always has a sha256 prefix")
}

pub(crate) fn from_output(output: &[u8]) -> OciDigest {
    let mut value = String::with_capacity(71);
    value.push_str("sha256:");
    for byte in output {
        use std::fmt::Write as _;
        write!(value, "{byte:02x}").expect("writing to String cannot fail");
    }
    OciDigest::parse(value).expect("SHA-256 output is a canonical OCI digest")
}
