//! Exact byte layouts of the classic `ifreq` and `rtentry` interface requests.

pub(super) const IFNAMSIZ: usize = 16;
pub(super) const IFREQ_DATA: usize = 24;
pub(super) const ARPHRD_ETHER: u16 = 1;
pub(super) const AF_INET: u16 = 2;
pub(super) const IFF_UP: i16 = 0x1;
pub(super) const IFF_RUNNING: i16 = 0x40;

#[repr(C)]
pub(super) struct IfReq {
    pub(super) name: [u8; IFNAMSIZ],
    pub(super) data: [u8; IFREQ_DATA],
}

#[repr(C)]
pub(super) struct RouteEntry {
    pub(super) pad1: u64,
    pub(super) dst: [u8; 16],
    pub(super) gateway: [u8; 16],
    pub(super) genmask: [u8; 16],
    pub(super) flags: u16,
    pub(super) pad2: i16,
    pub(super) pad3: u64,
    pub(super) tos: u8,
    pub(super) class: u8,
    pub(super) pad4: [i16; 3],
    pub(super) metric: i16,
    pub(super) dev: *mut libc::c_char,
    pub(super) mtu: u64,
    pub(super) window: u64,
    pub(super) irtt: u16,
}

pub(super) fn ifreq(name: &str, data: [u8; IFREQ_DATA]) -> IfReq {
    let mut request = IfReq {
        name: [0; IFNAMSIZ],
        data,
    };
    let bytes = name.as_bytes();
    request.name[..bytes.len().min(IFNAMSIZ - 1)]
        .copy_from_slice(&bytes[..bytes.len().min(IFNAMSIZ - 1)]);
    request
}

/// Encodes an IPv4 `sockaddr_in` with port zero into the request payload.
pub(super) fn inet_data(address: [u8; 4]) -> [u8; IFREQ_DATA] {
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&AF_INET.to_ne_bytes());
    data[4..8].copy_from_slice(&address);
    data
}

/// Encodes an Ethernet `sockaddr` carrying the MAC in `sa_data`.
pub(super) fn hwaddr_data(mac: [u8; 6]) -> [u8; IFREQ_DATA] {
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&ARPHRD_ETHER.to_ne_bytes());
    data[2..8].copy_from_slice(&mac);
    data
}

pub(super) fn flags_data(flags: i16) -> [u8; IFREQ_DATA] {
    let mut data = [0; IFREQ_DATA];
    data[..2].copy_from_slice(&flags.to_ne_bytes());
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_structure_layouts_are_exact() {
        assert_eq!(std::mem::size_of::<IfReq>(), 40);
        assert_eq!(std::mem::size_of::<RouteEntry>(), 120);
        assert_eq!(i32::from(AF_INET), libc::AF_INET);
        assert_eq!(i32::from(IFF_UP), libc::IFF_UP);
        assert_eq!(i32::from(IFF_RUNNING), libc::IFF_RUNNING);
    }

    #[test]
    fn requests_encode_names_addresses_and_flags_without_overflow() {
        let req = ifreq("a-very-long-interface-name", inet_data([10, 0, 0, 2]));
        assert_eq!(&req.name[..15], b"a-very-long-int");
        assert_eq!(req.name[15], 0);
        assert_eq!(&req.data[4..8], &[10, 0, 0, 2]);
        assert_eq!(u16::from_ne_bytes([req.data[0], req.data[1]]), AF_INET);

        let hw = hwaddr_data([1, 2, 3, 4, 5, 6]);
        assert_eq!(&hw[2..8], &[1, 2, 3, 4, 5, 6]);
        assert_eq!(flags_data(0x0041)[..2], 0x0041_i16.to_ne_bytes());
    }
}
