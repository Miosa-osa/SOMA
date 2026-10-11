//! What one bundle carried, read from the sandbox veth before teardown.
//!
//! The sandbox veth is the only way out of the bundle's namespace, and the forwarding chain has
//! already dropped everything the policy refuses by the time a frame reaches it. Its counters
//! therefore count exactly the bytes the guest sent to the public internet and received back,
//! with no ruleset counter to render and no extra kernel transaction on the create path.

use crate::{Error, Step, namespace::NetNamespace};

/// Bytes and packets one bundle carried over its life.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    /// Bytes the guest sent out of the bundle.
    pub bytes_out: u64,
    /// Bytes delivered to the guest.
    pub bytes_in: u64,
    /// Packets the guest sent out of the bundle.
    pub packets_out: u64,
    /// Packets delivered to the guest.
    pub packets_in: u64,
}

/// Reads the bundle's usage from inside its namespace.
///
/// # Errors
///
/// Returns the namespace-entry failure, or [`Error::Kernel`] at [`Step::LedgerRead`] when the
/// counters are absent or malformed.
pub(crate) fn read(namespace: &NetNamespace, veth: &str) -> Result<Usage, Error> {
    let veth = veth.to_owned();
    namespace.within(move || {
        // The thread-level view, because `within` moved only this thread into the namespace.
        let table = std::fs::read_to_string("/proc/thread-self/net/dev")
            .map_err(|error| Error::io(Step::LedgerRead, &error))?;
        parse(&table, &veth).ok_or(Error::Kernel {
            step: Step::LedgerRead,
            errno: 0,
        })
    })
}

/// Parses one interface's counters out of `/proc/net/dev` text.
///
/// The sandbox veth transmits what the guest sent out of the bundle and receives what the host
/// side delivers back, so transmit is the billable direction.
fn parse(table: &str, name: &str) -> Option<Usage> {
    let line = table
        .lines()
        .find_map(|line| line.trim_start().strip_prefix(name)?.strip_prefix(':'))?;
    let columns: Vec<u64> = line
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    // rx: bytes packets ...(6 more); tx: bytes packets ... (index 8 and 9).
    let (rx_bytes, rx_packets, tx_bytes, tx_packets) = (
        *columns.first()?,
        *columns.get(1)?,
        *columns.get(8)?,
        *columns.get(9)?,
    );
    Some(Usage {
        bytes_out: tx_bytes,
        bytes_in: rx_bytes,
        packets_out: tx_packets,
        packets_in: rx_packets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo:       0       0    0    0    0     0          0         0        0       0    0    0    0     0       0          0\n   vs0:    5000      40    0    0    0     0          0         0   123456     321    0    0    0     0       0          0\n";

    #[test]
    fn an_interface_is_read_by_exact_name() {
        let usage = parse(TABLE, "vs0").expect("vs0 present");
        assert_eq!(usage.bytes_in, 5000);
        assert_eq!(usage.packets_in, 40);
        assert_eq!(usage.bytes_out, 123_456);
        assert_eq!(usage.packets_out, 321);
        assert!(parse(TABLE, "vs").is_none(), "a prefix is not a name");
        assert!(parse(TABLE, "vs1").is_none());
    }
}
