//! Frame construction and checksum helpers for the guest stand-in.

// Frame arithmetic is over bounded, test-generated buffers below 2 KiB.
#![allow(clippy::cast_possible_truncation)]

use std::net::Ipv4Addr;

pub fn ethernet(destination: [u8; 6], source: [u8; 6], ethertype: u16) -> Vec<u8> {
    let mut frame = Vec::with_capacity(1518);
    frame.extend_from_slice(&destination);
    frame.extend_from_slice(&source);
    frame.extend_from_slice(&ethertype.to_be_bytes());
    frame
}

pub fn ethertype(frame: &[u8]) -> u16 {
    u16::from_be_bytes([frame[12], frame[13]])
}

pub fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in bytes.chunks(2) {
        let word = u16::from_be_bytes([chunk[0], *chunk.get(1).unwrap_or(&0)]);
        sum += u32::from(word);
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn transport_checksum(
    source: Ipv4Addr,
    destination: Ipv4Addr,
    protocol: u8,
    segment: &[u8],
) -> u16 {
    let mut pseudo = Vec::with_capacity(12 + segment.len());
    pseudo.extend_from_slice(&source.octets());
    pseudo.extend_from_slice(&destination.octets());
    pseudo.extend_from_slice(&[0, protocol]);
    pseudo.extend_from_slice(&(segment.len() as u16).to_be_bytes());
    pseudo.extend_from_slice(segment);
    checksum(&pseudo)
}
