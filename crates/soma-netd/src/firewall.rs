//! nftables ruleset generation for the sandbox namespace of one bundle.
//!
//! Version 1 renders one `inet` table per bundle as text for `nft -f -`.
//! Every chain has policy `drop`; the protected set is dropped before any accept in every
//! egress mode; DNS is admitted only to declared resolvers; the guest MAC and address are
//! bound to the TAP so the guest cannot spoof another lease.

use std::fmt::Write as _;

use crate::{NetworkIntent, ProtectedSet, ingress::PublishedPort, ipam::Lease};

pub mod host;
pub mod publish;

/// The kernel-facing names of one bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleNames {
    /// The `inet` table inside the sandbox namespace.
    pub sandbox_table: String,
    /// The `inet` table inside the host namespace.
    pub host_table: String,
    /// The `inet` table inside the host namespace that holds the published translations.
    pub publication_table: String,
    /// The TAP interface inside the sandbox namespace.
    pub tap: String,
    /// The veth end inside the sandbox namespace.
    pub sandbox_veth: String,
    /// The veth end inside the host namespace.
    pub host_veth: String,
}

impl BundleNames {
    /// Derives the names from the eight-character bundle prefix.
    #[must_use]
    pub fn new(short_hex: &str) -> Self {
        Self {
            sandbox_table: format!("soma_{short_hex}"),
            host_table: format!("somah_{short_hex}"),
            publication_table: format!("somap_{short_hex}"),
            tap: "tap0".to_owned(),
            sandbox_veth: "vs0".to_owned(),
            host_veth: format!("sv{short_hex}"),
        }
    }
}

/// Everything the sandbox ruleset depends on.
#[derive(Clone, Debug)]
pub struct SandboxRuleset<'a> {
    /// Kernel names.
    pub names: &'a BundleNames,
    /// The guest lease.
    pub lease: Lease,
    /// The guest MAC bound to the TAP.
    pub guest_mac: [u8; 6],
    /// The admitted intent.
    pub intent: &'a NetworkIntent,
    /// The complete protected set.
    pub protected: &'a ProtectedSet,
    /// The published mappings this ruleset admits inbound.
    ///
    /// It is empty while the bundle is sterile and while it is merely assigned; activation is
    /// the only step that renders the table again with the admitted mappings, so an inbound
    /// connection has nothing to match on until then.
    pub published: &'a [PublishedPort],
}

/// Formats one MAC for nftables.
#[must_use]
pub fn mac_text(mac: [u8; 6]) -> String {
    mac.iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub(crate) fn protected_sets(protected: &ProtectedSet, out: &mut String) {
    let (v4, v6): (Vec<_>, Vec<_>) = protected
        .entries()
        .iter()
        .map(|entry| entry.cidr)
        .partition(|cidr| cidr.is_v4());
    for (name, family, elements) in [
        ("protected4", "ipv4_addr", v4),
        ("protected6", "ipv6_addr", v6),
    ] {
        let _ = writeln!(out, "\tset {name} {{");
        let _ = writeln!(out, "\t\ttype {family}");
        let _ = writeln!(out, "\t\tflags interval");
        let _ = writeln!(out, "\t\tauto-merge");
        let list = elements
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "\t\telements = {{ {list} }}");
        let _ = writeln!(out, "\t}}");
    }
}

impl SandboxRuleset<'_> {
    /// Renders the complete `nft -f` text, starting with a flush of the same table.
    #[must_use]
    pub fn render(&self) -> String {
        let table = &self.names.sandbox_table;
        let tap = &self.names.tap;
        let veth = &self.names.sandbox_veth;
        let guest = self.lease.guest();
        let gateway = self.lease.host();
        let mac = mac_text(self.guest_mac);
        let mut out = String::new();
        let _ = writeln!(out, "table inet {table}");
        let _ = writeln!(out, "delete table inet {table}");
        let _ = writeln!(out, "table inet {table} {{");
        protected_sets(self.protected, &mut out);
        if self.intent.dns_allowed() {
            let list = self
                .intent
                .resolvers()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "\tset resolvers4 {{");
            let _ = writeln!(out, "\t\ttype ipv4_addr");
            let _ = writeln!(out, "\t\telements = {{ {list} }}");
            let _ = writeln!(out, "\t}}");
        }
        let _ = writeln!(out, "\tchain input {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook input priority filter; policy drop;"
        );
        let _ = writeln!(
            out,
            "\t\tiifname \"{tap}\" ether saddr {mac} ip saddr {guest} ip daddr {gateway} icmp type echo-request accept"
        );
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain forward {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook forward priority filter; policy drop;"
        );
        let _ = writeln!(out, "\t\tiifname \"{tap}\" ether saddr != {mac} drop");
        let _ = writeln!(out, "\t\tiifname \"{tap}\" meta nfproto ipv6 drop");
        let _ = writeln!(out, "\t\tiifname \"{tap}\" ip saddr != {guest} drop");
        // The guest's answer on a published port has to be admitted before the protected sets,
        // because the host translated the client's source into the bundle's own transit
        // address and every address the broker could translate to lies inside the private
        // space the protected sets deny. Those sets bound where the guest may open a
        // connection, and a packet in the reply direction of a conntrack entry that some other
        // party opened is not the guest opening anything: the spoofing drops above have
        // already fixed its source, the rule names one published endpoint, and an entry in
        // that direction can exist only because the inbound rule further down admitted it.
        for port in self.published {
            let _ = writeln!(
                out,
                "\t\tiifname \"{tap}\" oifname \"{veth}\" {} ct direction reply ct state established accept",
                port.reply_match()
            );
        }
        let _ = writeln!(out, "\t\tiifname \"{tap}\" ip daddr @protected4 drop");
        let _ = writeln!(out, "\t\tiifname \"{tap}\" ip6 daddr @protected6 drop");
        if self.intent.dns_allowed() {
            let _ = writeln!(
                out,
                "\t\tiifname \"{tap}\" oifname \"{veth}\" ip daddr @resolvers4 udp dport 53 accept"
            );
            let _ = writeln!(
                out,
                "\t\tiifname \"{tap}\" oifname \"{veth}\" ip daddr @resolvers4 tcp dport 53 accept"
            );
        }
        let _ = writeln!(out, "\t\tiifname \"{tap}\" udp dport 53 drop");
        let _ = writeln!(out, "\t\tiifname \"{tap}\" tcp dport 53 drop");
        if self.intent.egress().forwards() {
            let _ = writeln!(
                out,
                "\t\tiifname \"{tap}\" oifname \"{veth}\" ct state new,established accept"
            );
        }
        // An inbound connection is new in this namespace even though the host already
        // translated it, because conntrack is per namespace, so the published endpoints are
        // named explicitly here rather than recognised by translation status. Admitting the
        // inbound half here, after every drop, is enough; the guest's answer needs the earlier
        // rule instead.
        for port in self.published {
            let _ = writeln!(
                out,
                "\t\tiifname \"{veth}\" oifname \"{tap}\" {} ct state new,established accept",
                port.guest_match()
            );
        }
        let _ = writeln!(
            out,
            "\t\tiifname \"{veth}\" oifname \"{tap}\" ip daddr {guest} ct state established,related accept"
        );
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain output {{");
        let _ = writeln!(
            out,
            "\t\ttype filter hook output priority filter; policy drop;"
        );
        let _ = writeln!(
            out,
            "\t\toifname \"{tap}\" ip daddr {guest} ct state established,related accept"
        );
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "}}");
        out
    }
}

#[cfg(test)]
mod tests;
