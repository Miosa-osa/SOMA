//! nftables translation generation for the published ports of one bundle.
//!
//! Every published mapping lives in its own `inet` table, separate from the bundle's host
//! table, for two reasons. The host table is rendered once while the bundle is still sterile
//! and never names a publication, so nothing can be reachable before activation; and release
//! can then prove the mappings gone by asking for that one table by name, exactly as it proves
//! the host table and the host veth gone.
//!
//! Translation happens at both the prerouting and the output hook, because a host process
//! connecting to a published loopback endpoint never traverses prerouting. Both then leave
//! through the bundle's host veth, where a masquerade replaces the original source, so the
//! guest answers an address that is routable back to the host.

use std::fmt::Write as _;

use super::BundleNames;
use crate::ingress::PublishedPort;

/// Everything the publication table depends on.
#[derive(Clone, Debug)]
pub struct PublicationRuleset<'a> {
    /// Kernel names.
    pub names: &'a BundleNames,
    /// The mappings this activation installs; an empty slice renders no table at all.
    pub published: &'a [PublishedPort],
}

impl PublicationRuleset<'_> {
    /// Renders the complete `nft -f` text, starting with a flush of the same table.
    #[must_use]
    pub fn render(&self) -> String {
        let table = &self.names.publication_table;
        let veth = &self.names.host_veth;
        let mut out = String::new();
        let _ = writeln!(out, "table inet {table}");
        let _ = writeln!(out, "delete table inet {table}");
        let _ = writeln!(out, "table inet {table} {{");
        let _ = writeln!(out, "\tchain prerouting {{");
        let _ = writeln!(
            out,
            "\t\ttype nat hook prerouting priority dstnat; policy accept;"
        );
        for port in self.published {
            let _ = writeln!(out, "\t\t{} {}", port.host_match(), port.translation());
        }
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain output {{");
        let _ = writeln!(
            out,
            "\t\ttype nat hook output priority dstnat; policy accept;"
        );
        for port in self.published {
            let _ = writeln!(out, "\t\t{} {}", port.host_match(), port.translation());
        }
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "\tchain postrouting {{");
        let _ = writeln!(
            out,
            "\t\ttype nat hook postrouting priority srcnat; policy accept;"
        );
        for port in self.published {
            let _ = writeln!(
                out,
                "\t\toifname \"{veth}\" {} ct status dnat masquerade",
                port.guest_match()
            );
        }
        let _ = writeln!(out, "\t}}");
        let _ = writeln!(out, "}}");
        out
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use soma::TransportProtocol;

    use super::*;

    #[test]
    fn every_mapping_is_translated_at_both_hooks_and_masqueraded_on_the_way_out() {
        let names = BundleNames::new("deadbeef");
        let published = [PublishedPort::new(
            Some(Ipv4Addr::LOCALHOST),
            40_000,
            Ipv4Addr::new(10, 200, 0, 2),
            80,
            TransportProtocol::Tcp,
        )];
        let text = PublicationRuleset {
            names: &names,
            published: &published,
        }
        .render();
        assert!(text.starts_with("table inet somap_deadbeef\ndelete table inet somap_deadbeef\n"));
        assert_eq!(
            text.matches("ip daddr 127.0.0.1 tcp dport 40000 dnat ip to 10.200.0.2:80")
                .count(),
            2,
            "prerouting reaches an external client and output reaches a host process"
        );
        assert!(text.contains(
            "oifname \"svdeadbeef\" ip daddr 10.200.0.2 tcp dport 80 ct status dnat masquerade"
        ));
    }

    #[test]
    fn an_empty_publication_set_still_renders_only_empty_chains() {
        let names = BundleNames::new("deadbeef");
        let text = PublicationRuleset {
            names: &names,
            published: &[],
        }
        .render();
        assert!(!text.contains("dnat"));
    }
}
