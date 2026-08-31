//! A minimal guest stand-in that speaks raw Ethernet over the transferred TAP descriptor.
//!
//! It answers ARP for its own address, resolves the gateway, and can send one ICMP echo,
//! one TCP SYN, or one UDP datagram and report exactly what came back.

#![allow(unsafe_code)]
// Frame arithmetic is over bounded, test-generated buffers below 2 KiB.
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::{
    net::Ipv4Addr,
    os::fd::{AsRawFd, OwnedFd},
    time::{Duration, Instant},
};

use soma_guest::LaunchNetwork;

use super::codec::{checksum, ethernet, ethertype, transport_checksum};

mod inbound;

const ETH_ARP: u16 = 0x0806;
const ETH_IP: u16 = 0x0800;
const BROADCAST: [u8; 6] = [0xff; 6];

/// The outcome of one TCP SYN probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynOutcome {
    /// A SYN-ACK came back; the destination listener is reachable.
    SynAck,
    /// A RST came back; the destination is reachable but closed.
    Rst,
    /// Nothing came back inside the timeout.
    Silence,
}

pub struct Guest {
    tap: OwnedFd,
    mac: [u8; 6],
    ip: Ipv4Addr,
    gateway: Ipv4Addr,
    gateway_mac: Option<[u8; 6]>,
    sequence: u16,
}

impl Guest {
    pub fn new(tap: OwnedFd, launch: &LaunchNetwork) -> Self {
        Self {
            tap,
            mac: launch.mac(),
            ip: Ipv4Addr::from(launch.address()),
            gateway: Ipv4Addr::from(launch.gateway()),
            gateway_mac: None,
            sequence: 1,
        }
    }

    pub fn resolve_gateway(&mut self, timeout: Duration) -> Option<[u8; 6]> {
        let mut request = ethernet(BROADCAST, self.mac, ETH_ARP);
        request.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        request.extend_from_slice(&self.mac);
        request.extend_from_slice(&self.ip.octets());
        request.extend_from_slice(&[0; 6]);
        request.extend_from_slice(&self.gateway.octets());
        self.send(&request);
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            if frame.len() >= 42
                && ethertype(&frame) == ETH_ARP
                && frame[20..22] == [0, 2]
                && frame[28..32] == self.gateway.octets()
            {
                let mut mac = [0; 6];
                mac.copy_from_slice(&frame[22..28]);
                self.gateway_mac = Some(mac);
                return Some(mac);
            }
        }
        None
    }

    pub fn ping(&mut self, destination: Ipv4Addr, timeout: Duration) -> bool {
        let sequence = self.next_sequence();
        let mut icmp = vec![8, 0, 0, 0, 0x53, 0x4f];
        icmp.extend_from_slice(&sequence.to_be_bytes());
        icmp.extend_from_slice(b"soma-netd-live");
        let checksum = checksum(&icmp);
        icmp[2..4].copy_from_slice(&checksum.to_be_bytes());
        self.send_ip(destination, 1, &icmp);
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            if let Some((source, protocol, payload)) = self.ip_payload(&frame)
                && source == destination
                && protocol == 1
                && payload.len() >= 8
                && payload[0] == 0
                && payload[4..6] == [0x53, 0x4f]
                && payload[6..8] == sequence.to_be_bytes()
            {
                return true;
            }
        }
        false
    }

    pub fn tcp_syn(&mut self, destination: Ipv4Addr, port: u16, timeout: Duration) -> SynOutcome {
        let source_port = 40_000 + self.next_sequence();
        let mut tcp = Vec::with_capacity(20);
        tcp.extend_from_slice(&source_port.to_be_bytes());
        tcp.extend_from_slice(&port.to_be_bytes());
        tcp.extend_from_slice(&0x1234_5678_u32.to_be_bytes());
        tcp.extend_from_slice(&0_u32.to_be_bytes());
        tcp.extend_from_slice(&[0x50, 0x02, 0xff, 0xff, 0, 0, 0, 0]);
        let sum = transport_checksum(self.ip, destination, 6, &tcp);
        tcp[16..18].copy_from_slice(&sum.to_be_bytes());
        self.send_ip(destination, 6, &tcp);
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            if let Some((source, protocol, payload)) = self.ip_payload(&frame)
                && source == destination
                && protocol == 6
                && payload.len() >= 20
                && u16::from_be_bytes([payload[0], payload[1]]) == port
                && u16::from_be_bytes([payload[2], payload[3]]) == source_port
            {
                let flags = payload[13];
                if flags & 0x12 == 0x12 {
                    let their_seq =
                        u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                    self.tcp_control(
                        destination,
                        port,
                        source_port,
                        their_seq.wrapping_add(1),
                        0x10,
                    );
                    self.tcp_control(
                        destination,
                        port,
                        source_port,
                        their_seq.wrapping_add(1),
                        0x14,
                    );
                    return SynOutcome::SynAck;
                }
                if flags & 0x04 != 0 {
                    return SynOutcome::Rst;
                }
            }
        }
        SynOutcome::Silence
    }

    pub fn udp_probe(
        &mut self,
        destination: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Option<Vec<u8>> {
        let source_port = 50_000 + self.next_sequence();
        let payload = b"soma-netd-dns-probe";
        let length = (8 + payload.len()) as u16;
        let mut udp = Vec::with_capacity(8 + payload.len());
        udp.extend_from_slice(&source_port.to_be_bytes());
        udp.extend_from_slice(&port.to_be_bytes());
        udp.extend_from_slice(&length.to_be_bytes());
        udp.extend_from_slice(&[0, 0]);
        udp.extend_from_slice(payload);
        let sum = transport_checksum(self.ip, destination, 17, &udp);
        udp[6..8].copy_from_slice(&sum.to_be_bytes());
        self.send_ip(destination, 17, &udp);
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            if let Some((source, protocol, payload)) = self.ip_payload(&frame)
                && source == destination
                && protocol == 17
                && payload.len() >= 8
                && u16::from_be_bytes([payload[0], payload[1]]) == port
                && u16::from_be_bytes([payload[2], payload[3]]) == source_port
            {
                return Some(payload[8..].to_vec());
            }
        }
        None
    }

    fn tcp_control(
        &mut self,
        destination: Ipv4Addr,
        port: u16,
        source_port: u16,
        ack: u32,
        flags: u8,
    ) {
        let mut tcp = Vec::with_capacity(20);
        tcp.extend_from_slice(&source_port.to_be_bytes());
        tcp.extend_from_slice(&port.to_be_bytes());
        tcp.extend_from_slice(&0x1234_5679_u32.to_be_bytes());
        tcp.extend_from_slice(&ack.to_be_bytes());
        tcp.extend_from_slice(&[0x50, flags, 0xff, 0xff, 0, 0, 0, 0]);
        let sum = transport_checksum(self.ip, destination, 6, &tcp);
        tcp[16..18].copy_from_slice(&sum.to_be_bytes());
        self.send_ip(destination, 6, &tcp);
    }

    fn next_sequence(&mut self) -> u16 {
        self.sequence = self.sequence.wrapping_add(1);
        self.sequence
    }

    fn send_ip(&mut self, destination: Ipv4Addr, protocol: u8, payload: &[u8]) {
        let Some(gateway_mac) = self.gateway_mac else {
            return;
        };
        let mut frame = ethernet(gateway_mac, self.mac, ETH_IP);
        let total = (20 + payload.len()) as u16;
        let mut header = vec![0x45, 0, 0, 0, 0, 0, 0x40, 0, 64, protocol, 0, 0];
        header[2..4].copy_from_slice(&total.to_be_bytes());
        header.extend_from_slice(&self.ip.octets());
        header.extend_from_slice(&destination.octets());
        let sum = checksum(&header);
        header[10..12].copy_from_slice(&sum.to_be_bytes());
        frame.extend_from_slice(&header);
        frame.extend_from_slice(payload);
        self.send(&frame);
    }

    fn ip_payload<'a>(&self, frame: &'a [u8]) -> Option<(Ipv4Addr, u8, &'a [u8])> {
        if frame.len() < 34 || ethertype(frame) != ETH_IP || frame[14] >> 4 != 4 {
            return None;
        }
        let header_len = usize::from(frame[14] & 0x0f) * 4;
        let destination = Ipv4Addr::new(frame[30], frame[31], frame[32], frame[33]);
        if destination != self.ip {
            return None;
        }
        let source = Ipv4Addr::new(frame[26], frame[27], frame[28], frame[29]);
        Some((source, frame[23], &frame[14 + header_len..]))
    }

    fn send(&self, frame: &[u8]) {
        // SAFETY: `frame` is a valid buffer for its full length; a failed write is reported
        // by the probe timing out.
        let _ = unsafe { libc::write(self.tap.as_raw_fd(), frame.as_ptr().cast(), frame.len()) };
    }

    fn recv_until(&self, deadline: Instant) -> Option<Vec<u8>> {
        loop {
            let remaining = deadline.checked_duration_since(Instant::now())?;
            let mut poll = libc::pollfd {
                fd: self.tap.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: `poll` receives one valid `pollfd` and a count of one.
            let ready = unsafe { libc::poll(&raw mut poll, 1, remaining.as_millis() as i32) };
            if ready <= 0 {
                return None;
            }
            let mut buffer = vec![0_u8; 2048];
            // SAFETY: `buffer` is a valid writable buffer of exactly the passed length.
            let read = unsafe {
                libc::read(
                    self.tap.as_raw_fd(),
                    buffer.as_mut_ptr().cast(),
                    buffer.len(),
                )
            };
            if read <= 0 {
                return None;
            }
            buffer.truncate(read as usize);
            if self.answer_arp(&buffer) {
                continue;
            }
            return Some(buffer);
        }
    }

    fn answer_arp(&self, frame: &[u8]) -> bool {
        if frame.len() < 42 || ethertype(frame) != ETH_ARP || frame[20..22] != [0, 1] {
            return false;
        }
        if frame[38..42] != self.ip.octets() {
            return true;
        }
        let mut requester = [0; 6];
        requester.copy_from_slice(&frame[22..28]);
        let mut reply = ethernet(requester, self.mac, ETH_ARP);
        reply.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
        reply.extend_from_slice(&self.mac);
        reply.extend_from_slice(&self.ip.octets());
        reply.extend_from_slice(&frame[22..32]);
        self.send(&reply);
        true
    }
}
