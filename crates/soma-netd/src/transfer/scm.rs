//! `sendmsg` and `recvmsg` with exactly one `SCM_RIGHTS` descriptor.

#![allow(unsafe_code)]
// Socket ABI values are fixed-width by definition; the casts below convert `libc` constants
// and structure sizes whose ranges are bounded by the kernel structures they describe.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_ptr_alignment
)]

use std::{
    mem,
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    ptr,
};

use super::{MAX_HEADER, TransferHeader};
use crate::{Error, Step, TransferRejection};

const CONTROL_LEN: usize = 32;

/// Sends the header and one descriptor as one sequenced packet.
///
/// The send never blocks, so a peer whose queue is full refuses the transfer instead of
/// stalling the single-threaded broker, and a partial send is refused rather than reported as
/// a transfer.
///
/// # Errors
///
/// Returns [`Error::Kernel`] at [`Step::SendMsg`] when the kernel refuses the packet.
pub fn send_tap(
    socket: BorrowedFd<'_>,
    header: &TransferHeader,
    tap: BorrowedFd<'_>,
) -> Result<(), Error> {
    let mut payload = header.encode();
    let mut iov = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    let mut control = [0_u8; CONTROL_LEN];
    // SAFETY: `msghdr` is a plain C aggregate for which all-zero bytes are valid.
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = cmsg_space();
    // SAFETY: the control buffer is at least `CMSG_SPACE(sizeof int)` bytes, `CMSG_FIRSTHDR`
    // therefore returns a valid header pointer, and `CMSG_DATA` points inside the buffer.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const message);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = cmsg_len();
        ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>(), tap.as_raw_fd());
    }
    // SAFETY: every pointer inside `message` references live locals for the call.
    let sent = unsafe {
        libc::sendmsg(
            socket.as_raw_fd(),
            &raw const message,
            libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
        )
    };
    if sent < 0 || sent as usize != MAX_HEADER {
        return Err(Error::kernel(Step::SendMsg));
    }
    Ok(())
}

/// Receives exactly one header and one descriptor.
///
/// # Errors
///
/// Returns [`Error::Transfer`] for a hostile packet or [`Error::Kernel`] at
/// [`Step::RecvMsg`].
/// Every received descriptor is closed when the packet is rejected.
pub fn receive_tap(socket: BorrowedFd<'_>) -> Result<(TransferHeader, OwnedFd), Error> {
    let mut payload = [0_u8; MAX_HEADER + 1];
    let mut iov = libc::iovec {
        iov_base: payload.as_mut_ptr().cast(),
        iov_len: payload.len(),
    };
    let mut control = [0_u8; CONTROL_LEN * 4];
    // SAFETY: `msghdr` is a plain C aggregate for which all-zero bytes are valid.
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len();
    // SAFETY: every pointer inside `message` references live locals sized as declared.
    let received =
        unsafe { libc::recvmsg(socket.as_raw_fd(), &raw mut message, libc::MSG_CMSG_CLOEXEC) };
    if received < 0 {
        return Err(Error::kernel(Step::RecvMsg));
    }
    let descriptors = collect_descriptors(&message);
    if message.msg_flags & libc::MSG_CTRUNC != 0 {
        return Err(Error::Transfer(TransferRejection::ControlShort));
    }
    let header = TransferHeader::decode(&payload[..received as usize])?;
    if descriptors.len() != 1 {
        return Err(Error::Transfer(TransferRejection::DescriptorCount));
    }
    let tap = descriptors
        .into_iter()
        .next()
        .ok_or(Error::Transfer(TransferRejection::DescriptorCount))?;
    Ok((header, tap))
}

fn collect_descriptors(message: &libc::msghdr) -> Vec<OwnedFd> {
    let mut owned = Vec::new();
    // SAFETY: `CMSG_FIRSTHDR` and `CMSG_NXTHDR` walk only inside the control buffer bounded by
    // `msg_controllen`, and each descriptor read is within the header's declared length.
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(message);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let data = libc::CMSG_DATA(cmsg);
                let bytes = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                for index in 0..bytes / mem::size_of::<libc::c_int>() {
                    let raw = ptr::read_unaligned(data.cast::<libc::c_int>().add(index));
                    owned.push(OwnedFd::from_raw_fd(raw));
                }
            }
            cmsg = libc::CMSG_NXTHDR(message, cmsg);
        }
    }
    owned
}

fn cmsg_space() -> usize {
    // SAFETY: `CMSG_SPACE` is a pure arithmetic macro.
    unsafe { libc::CMSG_SPACE(mem::size_of::<libc::c_int>() as u32) as usize }
}

fn cmsg_len() -> usize {
    // SAFETY: `CMSG_LEN` is a pure arithmetic macro.
    unsafe { libc::CMSG_LEN(mem::size_of::<libc::c_int>() as u32) as usize }
}

/// Creates one connected `SOCK_SEQPACKET` pair for tests and the daemon.
///
/// # Errors
///
/// Returns [`Error::Kernel`] at [`Step::Socket`].
pub fn seqpacket_pair() -> Result<(OwnedFd, OwnedFd), Error> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is a valid two-element array that `socketpair` fills.
    let result = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    if result != 0 {
        return Err(Error::kernel(Step::Socket));
    }
    // SAFETY: both descriptors are freshly created and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write as _, os::fd::AsFd};

    use super::*;
    use crate::{CleanupGeneration, transfer::tests::header};

    #[test]
    fn one_descriptor_with_a_valid_header_is_accepted() {
        let (left, right) = seqpacket_pair().expect("pair");
        let file = tempfile::tempfile().expect("file");
        send_tap(left.as_fd(), &header(), file.as_fd()).expect("sent");
        let (received, fd) = receive_tap(right.as_fd()).expect("received");
        assert_eq!(received, header());
        let mut copy = File::from(fd);
        copy.write_all(b"ok").expect("writable descriptor");
    }

    #[test]
    fn a_header_without_a_descriptor_is_rejected() {
        let (left, right) = seqpacket_pair().expect("pair");
        let bytes = header().encode();
        // SAFETY: `bytes` is a valid buffer for its full length.
        let sent = unsafe { libc::send(left.as_raw_fd(), bytes.as_ptr().cast(), bytes.len(), 0) };
        assert_eq!(sent as usize, bytes.len());
        assert_eq!(
            receive_tap(right.as_fd()).expect_err("no descriptor"),
            Error::Transfer(TransferRejection::DescriptorCount)
        );
    }

    #[test]
    fn a_wrong_header_is_rejected_and_the_descriptor_is_closed() {
        let (left, right) = seqpacket_pair().expect("pair");
        let file = tempfile::tempfile().expect("file");
        let mut bad = header();
        bad.generation = CleanupGeneration::new(1).expect("g");
        let mut bytes = bad.encode();
        bytes[0] = b'X';
        let mut iov = libc::iovec {
            iov_base: bytes.as_mut_ptr().cast(),
            iov_len: bytes.len(),
        };
        let mut control = [0_u8; CONTROL_LEN];
        // SAFETY: `msghdr` is a plain C aggregate for which all-zero bytes are valid.
        let mut message: libc::msghdr = unsafe { mem::zeroed() };
        message.msg_iov = &raw mut iov;
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = cmsg_space();
        // SAFETY: the control buffer holds one complete `SCM_RIGHTS` header.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const message);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = cmsg_len();
            ptr::write_unaligned(
                libc::CMSG_DATA(cmsg).cast::<libc::c_int>(),
                file.as_raw_fd(),
            );
            assert!(libc::sendmsg(left.as_raw_fd(), &raw const message, 0) > 0);
        }
        assert_eq!(
            receive_tap(right.as_fd()).expect_err("bad magic"),
            Error::Transfer(TransferRejection::BadMagic)
        );
    }
}
