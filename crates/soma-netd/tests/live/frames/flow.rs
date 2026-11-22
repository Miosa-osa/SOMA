//! The guest stand-in's held flows: a TCP connection opened once and probed again later.
//!
//! An egress change is only proved if it also governs flows that already existed, so the
//! stand-in can complete a handshake, keep the flow, and later ask whether it still passes.

use std::{
    net::Ipv4Addr,
    time::{Duration, Instant},
};

use super::Guest;
use crate::live::codec::transport_checksum;

/// A TCP flow [`Guest::tcp_open`] left open.
#[derive(Debug)]
pub struct OpenFlow {
    destination: Ipv4Addr,
    port: u16,
    source_port: u16,
    ack: u32,
    sequence: u32,
}

impl Guest {
    /// Completes a TCP handshake and leaves the flow open, so a later probe can say whether the
    /// flow is still admitted.
    pub fn tcp_open(
        &mut self,
        destination: Ipv4Addr,
        port: u16,
        timeout: Duration,
    ) -> Option<OpenFlow> {
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
                && payload[13] & 0x12 == 0x12
            {
                let ack = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]])
                    .wrapping_add(1);
                self.tcp_control(destination, port, source_port, ack, 0x10);
                return Some(OpenFlow {
                    destination,
                    port,
                    source_port,
                    ack,
                    sequence: 0x1234_5679,
                });
            }
        }
        None
    }

    /// Sends one data segment on a flow opened earlier and reports whether the destination
    /// answered it. A flow the network no longer admits is silent.
    pub fn tcp_data_answered(&mut self, flow: &mut OpenFlow, timeout: Duration) -> bool {
        let payload = b"x";
        let expected_ack = flow.sequence.wrapping_add(1);
        let mut tcp = Vec::with_capacity(21);
        tcp.extend_from_slice(&flow.source_port.to_be_bytes());
        tcp.extend_from_slice(&flow.port.to_be_bytes());
        tcp.extend_from_slice(&flow.sequence.to_be_bytes());
        tcp.extend_from_slice(&flow.ack.to_be_bytes());
        tcp.extend_from_slice(&[0x50, 0x18, 0xff, 0xff, 0, 0, 0, 0]);
        tcp.extend_from_slice(payload);
        let sum = transport_checksum(self.ip, flow.destination, 6, &tcp);
        tcp[16..18].copy_from_slice(&sum.to_be_bytes());
        self.send_ip(flow.destination, 6, &tcp);
        let deadline = Instant::now() + timeout;
        while let Some(frame) = self.recv_until(deadline) {
            if let Some((source, protocol, answer)) = self.ip_payload(&frame)
                && source == flow.destination
                && protocol == 6
                && answer.len() >= 20
                && u16::from_be_bytes([answer[0], answer[1]]) == flow.port
                && u16::from_be_bytes([answer[2], answer[3]]) == flow.source_port
                && answer[13] & 0x14 == 0x10
                && u32::from_be_bytes([answer[8], answer[9], answer[10], answer[11]])
                    == expected_ack
            {
                flow.sequence = expected_ack;
                return true;
            }
        }
        false
    }
}
