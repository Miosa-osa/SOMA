//! How a change of egress class renders: what a close removes and what an open admits.

use std::net::Ipv4Addr;

use super::{chain, every_protected_is_dropped_before_any_accept, render};
use crate::EgressClass;

#[test]
fn an_intent_that_denies_dns_closes_port_53_before_egress() {
    let ruleset = render(EgressClass::PublicInternet, Vec::new());
    let forward = chain(&ruleset, "forward");
    let drop = forward.find("udp dport 53 drop").expect("udp 53 drop");
    assert!(forward.contains("tcp dport 53 drop"));
    let egress = forward
        .find("oifname \"vs0\" ct state new,established accept")
        .expect("egress");
    assert!(drop < egress);
}

#[test]
fn closing_egress_removes_every_rule_that_lets_a_flow_continue() {
    let resolvers = vec![Ipv4Addr::new(1, 1, 1, 1)];
    let open = render(EgressClass::PublicInternet, resolvers);
    let closed = render(EgressClass::Denied, Vec::new());
    // The accept for new and established flows is the rule a close must remove: with it gone
    // the forward chain's drop policy also cuts flows that were already established.
    let accept = "ct state new,established accept";
    assert!(chain(&open, "forward").contains(accept));
    assert!(!chain(&closed, "forward").contains(accept));
    // The closed table is a complete replacement, not a patch on the open one.
    assert!(closed.starts_with("table inet soma_0a0b0c0d\ndelete table inet soma_0a0b0c0d\n"));
    assert!(chain(&closed, "forward").contains("policy drop;"));
}

#[test]
fn closing_egress_blocks_dns_and_opening_with_dns_admits_only_the_declared_resolvers() {
    let resolvers = vec![Ipv4Addr::new(1, 1, 1, 1)];
    let open = render(EgressClass::PublicInternet, resolvers);
    let closed = render(EgressClass::Denied, Vec::new());
    assert!(open.contains("set resolvers4"));
    assert!(chain(&open, "forward").contains("@resolvers4 udp dport 53 accept"));
    assert!(!closed.contains("resolvers4"));
    // Without DNS, name resolution is refused outright on both transports.
    assert!(chain(&closed, "forward").contains("udp dport 53 drop"));
    assert!(chain(&closed, "forward").contains("tcp dport 53 drop"));
    // An open table with DNS denied keeps port 53 closed too.
    let open_without_dns = render(EgressClass::PublicInternet, Vec::new());
    assert!(chain(&open_without_dns, "forward").contains("udp dport 53 drop"));
    assert!(!open_without_dns.contains("resolvers4"));
}

#[test]
fn a_change_of_egress_keeps_the_protected_set_and_the_published_ports_rendering() {
    let open = render(EgressClass::PublicInternet, vec![Ipv4Addr::new(1, 1, 1, 1)]);
    let closed = render(EgressClass::Denied, Vec::new());
    for ruleset in [&open, &closed] {
        every_protected_is_dropped_before_any_accept(ruleset);
    }
}
