//! nftables ruleset generation for the host-namespace side of one bundle.
//!
//! The host table binds the bundle's conntrack zone to its veth, masquerades the guest lease
//! toward the uplink, drops the protected set a second time, refuses spoofed sources, and
//! admits no unsolicited traffic toward the guest until the ingress slice exists.

use std::fmt::Write as _;

use super::{BundleNames, protected_sets};
use crate::{ConntrackZone, InterfaceName, ProtectedSet, ipam::Lease};

/// Everything the host ruleset depends on.
#[derive(Clone, Debug)]
pub struct HostRuleset<'a> {
    /// Kernel names.
    pub names: &'a BundleNames,
    /// The guest lease.
    pub lease: Lease,
    /// The host uplink interface.
    pub uplink: &'a InterfaceName,
    /// The bundle's conntrack zone.
    pub zone: ConntrackZone,
    /// The complete protected set.
    pub protected: &'a ProtectedSet,
}

impl HostRuleset<'_> {
    /// Renders the complete `nft -f` text, starting with a flush of the same table.
    #[must_use]
    pub fn render(&self) -> String {
        let table = &self.names.host_table;
        let veth = &self.names.host_veth;
        let uplink = self.uplink.as_str();
        let guest = self.lease.guest();
        let zone = self.zone.get();
        let mut out = String::new();
        let _ = writeln!(out, "table inet {table}");
        let _ = writeln!(out, "delete table inet {table}");
        let _ = writeln!(out, "table inet {table} {{");
        protected_sets(self.protected, &mut out);
        let _ = writeln!(out, "\tchain zone {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook prerouting priority raw; policy accept;"
        );
        let _ = writeln!(out, "\t\tiifname \"{veth}\" ct original zone set {zone}");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain forward {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook forward priority filter; policy accept;"
        );
        let _ = writeln!(out, "\t\tiifname \"{veth}\" ip saddr != {guest} drop");
        let _ = writeln!(out, "\t\tiifname \"{veth}\" meta nfproto ipv6 drop");
        let _ = writeln!(out, "\t\tiifname \"{veth}\" ip daddr @protected4 drop");
        let _ = writeln!(out, "\t\tiifname \"{veth}\" ip6 daddr @protected6 drop");
        let _ = writeln!(out, "\t\tiifname \"{veth}\" oifname != \"{uplink}\" drop");
        let _ = writeln!(
            out,
            "\t\toifname \"{veth}\" ct state new,invalid,untracked drop"
        );
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain input {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook input priority filter; policy accept;"
        );
        let _ = writeln!(out, "\t\tiifname \"{veth}\" drop");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain nat {{");
        let _ = writeln!(
            out,
            "\t\ttype nat hook postrouting priority srcnat; policy accept;"
        );
        let _ = writeln!(out, "\t\toifname \"{uplink}\" ip saddr {guest} masquerade");
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "}}");
        out
    }
}
