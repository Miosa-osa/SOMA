//! Exact byte layouts of the classic `ifreq` and `rtentry` interface requests.
//!
//! These layouts are transcribed for the Linux `x86_64` ABI only, which is the ABI
//! `super::target::VERIFIED` names and the only one the binary is built for.
//! The constant assertions below fail the build on an ABI whose C integer widths, pointer
//! width, or byte order differ, and `super::target::require` refuses at run time every ABI
//! that shares those widths but not the layouts.

/// The C integer widths, pointer width, and byte order these layouts assume.
const _: () = {
    assert!(size_of::<libc::c_ulong>() == 8);
    assert!(size_of::<libc::c_short>() == 2);
    assert!(size_of::<libc::c_ushort>() == 2);
    assert!(size_of::<libc::c_uchar>() == 1);
    assert!(size_of::<*mut libc::c_char>() == 8);
    assert!(align_of::<*mut libc::c_char>() == 8);
    assert!(cfg!(target_endian = "little"));
};

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

    fn offset_of<T>(base: &T, field: *const u8) -> usize {
        let start: *const u8 = std::ptr::from_ref(base).cast();
        // SAFETY: `field` points inside `base`, so the difference is a valid byte offset.
        usize::try_from(unsafe { field.offset_from(start) }).expect("field lies inside the struct")
    }

    fn route() -> RouteEntry {
        RouteEntry {
            pad1: 0,
            dst: [0; 16],
            gateway: [0; 16],
            genmask: [0; 16],
            flags: 0,
            pad2: 0,
            pad3: 0,
            tos: 0,
            class: 0,
            pad4: [0; 3],
            metric: 0,
            dev: std::ptr::null_mut(),
            mtu: 0,
            window: 0,
            irtt: 0,
        }
    }

    #[test]
    fn kernel_structure_layouts_are_exact() {
        assert_eq!(std::mem::size_of::<IfReq>(), 40);
        assert_eq!(std::mem::align_of::<IfReq>(), 1);
        assert_eq!(std::mem::size_of::<RouteEntry>(), 120);
        assert_eq!(std::mem::align_of::<RouteEntry>(), 8);
        assert_eq!(i32::from(AF_INET), libc::AF_INET);
        assert_eq!(i32::from(IFF_UP), libc::IFF_UP);
        assert_eq!(i32::from(IFF_RUNNING), libc::IFF_RUNNING);
        assert_eq!(IFNAMSIZ, libc::IFNAMSIZ);
        assert_eq!(std::mem::size_of::<libc::sockaddr>(), 16);
    }

    #[test]
    fn every_route_entry_field_sits_at_its_x86_64_offset() {
        let entry = route();
        let offsets = [
            offset_of(&entry, std::ptr::from_ref(&entry.pad1).cast()),
            offset_of(&entry, entry.dst.as_ptr()),
            offset_of(&entry, entry.gateway.as_ptr()),
            offset_of(&entry, entry.genmask.as_ptr()),
            offset_of(&entry, std::ptr::from_ref(&entry.flags).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.pad2).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.pad3).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.tos).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.class).cast()),
            offset_of(&entry, entry.pad4.as_ptr().cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.metric).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.dev).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.mtu).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.window).cast()),
            offset_of(&entry, std::ptr::from_ref(&entry.irtt).cast()),
        ];

        assert_eq!(
            offsets,
            [0, 8, 24, 40, 56, 58, 64, 72, 73, 74, 80, 88, 96, 104, 112]
        );
    }

    #[test]
    fn the_interface_request_places_the_payload_after_the_name() {
        let request = ifreq("eth0", [0xAB; IFREQ_DATA]);

        assert_eq!(offset_of(&request, request.name.as_ptr()), 0);
        assert_eq!(offset_of(&request, request.data.as_ptr()), IFNAMSIZ);
        assert_eq!(IFNAMSIZ + IFREQ_DATA, std::mem::size_of::<IfReq>());
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
