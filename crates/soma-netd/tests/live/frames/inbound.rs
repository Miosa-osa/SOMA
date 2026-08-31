//! The guest stand-in's inbound half: answering one connection that reached it from outside.
//!
//! Egress probes only need the guest to speak first. A published port is the opposite
//! direction, so the stand-in has to behave like a listening service for exactly one
//! handshake: it must see the translated SYN arrive on its own address and answer it well
//! enough that the client's kernel completes the connection.

use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};

use super::Guest;
use crate::live::codec::transport_checksum;

impl Guest {
    /// Answers one inbound TCP SYN on `port` and returns the source it came from.
    ///
    /// The reply is a bare SYN-ACK with no options, which is all the client's kernel needs to
    /// declare the connection established; nothing here is a TCP implementation.
    pub fn accept_syn(&mut self, port: u16, timeout: Duration) -> Option<Ipv4Addr> {
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            let Some((source, 6, segment)) = self.ip_payload(&frame) else {
                continue;
            };
            if segment.len() < 20 || u16::from_be_bytes([segment[2], segment[3]]) != port {
                continue;
            }
            // A SYN that is not also an ACK is the start of an inbound connection; anything
            // else on this port belongs to a handshake that already completed.
            if segment[13] & 0x12 != 0x02 {
                continue;
            }
            let client_port = u16::from_be_bytes([segment[0], segment[1]]);
            let client_sequence =
                u32::from_be_bytes([segment[4], segment[5], segment[6], segment[7]]);
            self.send_syn_ack(source, port, client_port, client_sequence.wrapping_add(1));
            return Some(source);
        }
        None
    }

    fn send_syn_ack(&mut self, client: Ipv4Addr, port: u16, client_port: u16, ack: u32) {
        let mut tcp = Vec::with_capacity(20);
        tcp.extend_from_slice(&port.to_be_bytes());
        tcp.extend_from_slice(&client_port.to_be_bytes());
        tcp.extend_from_slice(&0x736f_6d61_u32.to_be_bytes());
        tcp.extend_from_slice(&ack.to_be_bytes());
        tcp.extend_from_slice(&[0x50, 0x12, 0xff, 0xff, 0, 0, 0, 0]);
        let sum = transport_checksum(self.ip, client, 6, &tcp);
        tcp[16..18].copy_from_slice(&sum.to_be_bytes());
        self.send_ip(client, 6, &tcp);
    }
}
