//! Bounded transfer of verified open files to a child process.

#![allow(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_ptr_alignment,
    clippy::cast_sign_loss
)]

use std::{
    mem,
    os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd},
    ptr,
};

const MAGIC: &[u8; 8] = b"SOMAFDS\0";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 12;
const MAX_DESCRIPTORS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorHandoffError {
    Empty,
    TooMany,
    Kernel,
    Protocol,
}

/// Sends one bounded, ordered set of open files over a connected Unix socket.
///
/// # Errors
///
/// Returns a bounded validation or kernel transfer failure.
pub fn send_descriptors(
    socket: BorrowedFd<'_>,
    descriptors: &[BorrowedFd<'_>],
) -> Result<(), DescriptorHandoffError> {
    if descriptors.is_empty() {
        return Err(DescriptorHandoffError::Empty);
    }
    if descriptors.len() > MAX_DESCRIPTORS {
        return Err(DescriptorHandoffError::TooMany);
    }
    let mut header = [0_u8; HEADER_LEN];
    header[..8].copy_from_slice(MAGIC);
    header[8..10].copy_from_slice(&VERSION.to_be_bytes());
    let count = u16::try_from(descriptors.len()).map_err(|_| DescriptorHandoffError::TooMany)?;
    header[10..].copy_from_slice(&count.to_be_bytes());
    let mut iov = libc::iovec {
        iov_base: header.as_mut_ptr().cast(),
        iov_len: header.len(),
    };
    let mut control = control_words(descriptors.len());
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = cmsg_space(descriptors.len());
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const message);
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = cmsg_len(descriptors.len());
        let data = libc::CMSG_DATA(cmsg).cast::<libc::c_int>();
        for (index, descriptor) in descriptors.iter().enumerate() {
            ptr::write_unaligned(data.add(index), descriptor.as_raw_fd());
        }
    }
    let sent = unsafe { libc::sendmsg(socket.as_raw_fd(), &raw const message, libc::MSG_NOSIGNAL) };
    if sent == HEADER_LEN.cast_signed() {
        Ok(())
    } else {
        Err(DescriptorHandoffError::Kernel)
    }
}

/// Receives one bounded, ordered set of open files from a connected Unix socket.
///
/// # Errors
///
/// Returns a kernel or protocol failure and closes every descriptor already received.
pub fn receive_descriptors(socket: BorrowedFd<'_>) -> Result<Vec<OwnedFd>, DescriptorHandoffError> {
    // This socket remains in use for the JSON launch request after the handoff.
    // Reading even one probe byte beyond the fixed header would steal that request's first byte.
    let mut header = [0_u8; HEADER_LEN];
    let mut iov = libc::iovec {
        iov_base: header.as_mut_ptr().cast(),
        iov_len: header.len(),
    };
    let mut control = control_words(MAX_DESCRIPTORS);
    let mut message: libc::msghdr = unsafe { mem::zeroed() };
    message.msg_iov = &raw mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = control.len() * mem::size_of::<usize>();
    let received = unsafe {
        libc::recvmsg(
            socket.as_raw_fd(),
            &raw mut message,
            libc::MSG_CMSG_CLOEXEC | libc::MSG_WAITALL,
        )
    };
    if received < 0 {
        return Err(DescriptorHandoffError::Kernel);
    }
    let owned = collect_descriptors(&message);
    if usize::try_from(received).ok() != Some(HEADER_LEN)
        || message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0
        || &header[..8] != MAGIC
        || u16::from_be_bytes([header[8], header[9]]) != VERSION
    {
        return Err(DescriptorHandoffError::Protocol);
    }
    let declared = u16::from_be_bytes([header[10], header[11]]) as usize;
    if declared == 0 || declared > MAX_DESCRIPTORS || owned.len() != declared {
        return Err(DescriptorHandoffError::Protocol);
    }
    Ok(owned)
}

fn collect_descriptors(message: &libc::msghdr) -> Vec<OwnedFd> {
    let mut owned = Vec::new();
    unsafe {
        let mut cmsg = libc::CMSG_FIRSTHDR(message);
        while !cmsg.is_null() {
            if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
                let bytes = (*cmsg).cmsg_len.saturating_sub(cmsg_len(0));
                let count = bytes / mem::size_of::<libc::c_int>();
                let data = libc::CMSG_DATA(cmsg).cast::<libc::c_int>();
                for index in 0..count {
                    owned.push(OwnedFd::from_raw_fd(ptr::read_unaligned(data.add(index))));
                }
            }
            cmsg = libc::CMSG_NXTHDR(message, cmsg);
        }
    }
    owned
}

fn control_words(count: usize) -> Vec<usize> {
    vec![0; cmsg_space(count).div_ceil(mem::size_of::<usize>())]
}

const fn cmsg_space(count: usize) -> usize {
    unsafe { libc::CMSG_SPACE((count * mem::size_of::<libc::c_int>()) as u32) as usize }
}

const fn cmsg_len(count: usize) -> usize {
    unsafe { libc::CMSG_LEN((count * mem::size_of::<libc::c_int>()) as u32) as usize }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Read as _, os::fd::AsFd as _, os::unix::net::UnixStream};

    #[test]
    fn ordered_files_cross_one_handoff() {
        let (left, right) = UnixStream::pair().expect("socket pair");
        let first = File::open("/dev/null").expect("first file");
        let second = File::open("/dev/zero").expect("second file");

        super::send_descriptors(left.as_fd(), &[first.as_fd(), second.as_fd()]).expect("send");
        let received = super::receive_descriptors(right.as_fd()).expect("receive");

        assert_eq!(received.len(), 2);
        let mut zero = File::from(received.into_iter().nth(1).expect("second descriptor"));
        let mut byte = [1_u8];
        zero.read_exact(&mut byte).expect("read zero");
        assert_eq!(byte, [0]);
    }
}
