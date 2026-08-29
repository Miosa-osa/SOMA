//! Portable encoding of Linux `ioctl` request codes across C library targets.
//!
//! glibc declares the request operand as `unsigned long` while musl declares it as `int`, so
//! the fixed SOMA request constants are converted through one checked helper.

/// The request operand type expected by the target C library's `ioctl`.
#[cfg(target_env = "musl")]
pub type Request = libc::c_int;
/// The request operand type expected by the target C library's `ioctl`.
#[cfg(not(target_env = "musl"))]
pub type Request = libc::c_ulong;

/// Converts a kernel request code into the C library operand type.
///
/// A code that cannot be represented becomes `-1`, which the kernel rejects with `ENOTTY`.
#[must_use]
pub fn request(code: libc::c_ulong) -> Request {
    Request::try_from(code).unwrap_or(Request::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_request_codes_are_representable() {
        assert_eq!(i64::try_from(request(0x4008_5203)), Ok(0x4008_5203));
        assert_eq!(i64::try_from(request(0x890B)), Ok(0x890B));
        assert_eq!(i64::try_from(request(0x7b9)), Ok(0x7b9));
    }
}
