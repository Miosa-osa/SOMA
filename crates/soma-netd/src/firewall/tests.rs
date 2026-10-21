use std::net::Ipv4Addr;

use super::host::HostRuleset;
use super::*;
use crate::{
    ConntrackZone, EgressClass, InterfaceName, ProfileDigest, SubnetPlan,
    protected::CERTIFIED_DEFAULT,
};

fn lease() -> Lease {
    SubnetPlan::new(Ipv4Addr::new(10, 200, 0, 0), 16)
        .expect("plan")
        .lease(5)
        .expect("lease")
}

fn intent(egress: EgressClass, resolvers: Vec<Ipv4Addr>) -> NetworkIntent {
    NetworkIntent::new(egress, resolvers, Vec::new(), ProfileDigest([1; 32])).expect("intent")
}

fn render(egress: EgressClass, resolvers: Vec<Ipv4Addr>) -> String {
    render_with(egress, resolvers, &[])
}

fn render_with(
    egress: EgressClass,
    resolvers: Vec<Ipv4Addr>,
    published: &[PublishedPort],
) -> String {
    render_connecting(egress, resolvers, published, &[])
}

fn render_connecting(
    egress: EgressClass,
    resolvers: Vec<Ipv4Addr>,
    published: &[PublishedPort],
    connect_ports: &[std::ops::RangeInclusive<u16>],
) -> String {
    let names = BundleNames::new("0a0b0c0d");
    let intent = intent(egress, resolvers);
    let protected = ProtectedSet::certified_default();
    SandboxRuleset {
        names: &names,
        lease: lease(),
        guest_mac: [0x02, 1, 2, 3, 4, 5],
        intent: &intent,
        protected: &protected,
        published,
        connect_ports,
    }
    .render()
}

fn chain<'a>(ruleset: &'a str, name: &str) -> &'a str {
    let start = ruleset
        .find(&format!("\tchain {name} {{"))
        .expect("chain present");
    let end = ruleset[start..].find("\n\t}\n").expect("chain end") + start;
    &ruleset[start..end]
}

fn every_protected_is_dropped_before_any_accept(ruleset: &str) {
    for entry in CERTIFIED_DEFAULT {
        let text = entry.cidr.to_string();
        assert!(
            ruleset.contains(&text),
            "{text} ({:?}) missing from the protected sets",
            entry.reason
        );
    }
    for name in ["forward", "input", "output"] {
        let body = chain(ruleset, name);
        assert!(body.contains("policy drop;"), "{name} must default to drop");
    }
    let forward = chain(ruleset, "forward");
    let first_accept = forward.find(" accept").unwrap_or(forward.len());
    let drop4 = forward.find("ip daddr @protected4 drop").expect("v4 drop");
    let drop6 = forward.find("ip6 daddr @protected6 drop").expect("v6 drop");
    assert!(
        drop4 < first_accept,
        "protected4 drop must precede any accept"
    );
    assert!(
        drop6 < first_accept,
        "protected6 drop must precede any accept"
    );
    let spoof = forward
        .find("ether saddr != 02:01:02:03:04:05 drop")
        .expect("mac");
    assert!(spoof < first_accept);
    assert!(forward.find("ip saddr != 10.200.0.22 drop").expect("addr") < first_accept);
}

#[test]
fn denied_ruleset_matches_golden_and_accepts_no_forwarding() {
    let ruleset = render(EgressClass::Denied, Vec::new());
    every_protected_is_dropped_before_any_accept(&ruleset);
    let forward = chain(&ruleset, "forward");
    assert!(
        !forward.contains("oifname \"vs0\""),
        "denied mode forwards nothing"
    );
    assert!(!ruleset.contains("resolvers4"));
    let expected_input = "\tchain input {\n\t\ttype filter hook input priority filter; policy drop;\n\t\tiifname \"tap0\" ether saddr 02:01:02:03:04:05 ip saddr 10.200.0.22 ip daddr 10.200.0.21 icmp type echo-request accept";
    assert_eq!(chain(&ruleset, "input"), expected_input);
    assert!(ruleset.starts_with(
        "table inet soma_0a0b0c0d\ndelete table inet soma_0a0b0c0d\ntable inet soma_0a0b0c0d {\n"
    ));
}

#[test]
fn public_internet_ruleset_admits_declared_dns_and_egress_after_protection() {
    let ruleset = render(
        EgressClass::PublicInternet,
        vec![Ipv4Addr::new(1, 1, 1, 1), Ipv4Addr::new(9, 9, 9, 9)],
    );
    every_protected_is_dropped_before_any_accept(&ruleset);
    assert!(ruleset.contains(
        "\tset resolvers4 {\n\t\ttype ipv4_addr\n\t\telements = { 1.1.1.1, 9.9.9.9 }\n\t}\n"
    ));
    let forward = chain(&ruleset, "forward");
    let dns = forward
        .find("ip daddr @resolvers4 udp dport 53 accept")
        .expect("dns accept");
    let egress = forward
        .find("iifname \"tap0\" oifname \"vs0\" ct state new,established accept")
        .expect("egress");
    assert!(dns < egress);
    // Any public resolver is reachable on 53, so nothing closes the port after the protected
    // set has been dropped.
    assert!(!forward.contains("dport 53 drop"));
}

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
fn outbound_smtp_is_dropped_before_egress_is_admitted_in_every_forwarding_mode() {
    for class in [EgressClass::PublicInternet, EgressClass::Unrestricted] {
        let ruleset = render(class, Vec::new());
        let forward = chain(&ruleset, "forward");
        let smtp = forward.find("tcp dport 25 drop").expect("smtp drop");
        let egress = forward
            .find("oifname \"vs0\" ct state new,established accept")
            .expect("egress");
        assert!(
            smtp < egress,
            "mail must be refused before anything is admitted"
        );
    }
}

#[test]
fn unrestricted_ruleset_still_drops_the_protected_set_first() {
    let ruleset = render(EgressClass::Unrestricted, Vec::new());
    every_protected_is_dropped_before_any_accept(&ruleset);
    assert!(chain(&ruleset, "forward").contains("oifname \"vs0\" ct state new,established accept"));
    assert!(!ruleset.contains("dport 53 accept"));
}

#[test]
fn host_ruleset_binds_zone_masquerade_and_protection() {
    let names = BundleNames::new("0a0b0c0d");
    let uplink = InterfaceName::new("uplink0").expect("name");
    let protected = ProtectedSet::certified_default();
    let ruleset = HostRuleset {
        names: &names,
        lease: lease(),
        uplink: &uplink,
        zone: ConntrackZone::new(6).expect("zone"),
        protected: &protected,
    }
    .render();
    assert!(ruleset.contains("iifname \"sv0a0b0c0d\" ct original zone set 6"));
    assert!(ruleset.contains("oifname \"uplink0\" ip saddr 10.200.0.22 masquerade"));
    assert!(ruleset.contains("iifname \"sv0a0b0c0d\" ip daddr @protected4 drop"));
    assert!(ruleset.contains("oifname \"sv0a0b0c0d\" ct state new,invalid,untracked drop"));
    assert!(ruleset.contains("iifname \"sv0a0b0c0d\" oifname != \"uplink0\" drop"));
    for entry in CERTIFIED_DEFAULT {
        assert!(ruleset.contains(&entry.cidr.to_string()));
    }
    assert_eq!(names.host_veth.len(), 10);
    assert_eq!(
        mac_text([0xde, 0xad, 0xbe, 0xef, 0, 1]),
        "de:ad:be:ef:00:01"
    );
}

#[test]
fn a_published_port_is_admitted_only_on_its_own_endpoint_and_in_both_directions() {
    let published = [PublishedPort::new(
        Some(Ipv4Addr::LOCALHOST),
        40_000,
        Ipv4Addr::new(10, 200, 0, 22),
        80,
        soma::TransportProtocol::Tcp,
    )];
    let ruleset = render_with(EgressClass::Denied, Vec::new(), &published);
    let forward = chain(&ruleset, "forward");
    assert!(
        forward.contains(
            "iifname \"vs0\" oifname \"tap0\" ip daddr 10.200.0.22 tcp dport 80 ct state new,established accept"
        ),
        "the inbound half must be admitted even though egress is denied"
    );
    assert!(
        forward.contains(
            "iifname \"tap0\" oifname \"vs0\" ip saddr 10.200.0.22 tcp sport 80 ct direction reply ct state established accept"
        ),
        "the guest may answer on the published endpoint and nowhere else"
    );
    assert!(
        !forward.contains("iifname \"tap0\" oifname \"vs0\" ct state new,established accept"),
        "publishing a port must not open general egress"
    );
    let spoof = forward
        .find("ip saddr != 10.200.0.22 drop")
        .expect("source spoofing is refused");
    let reply = forward.find("ct direction reply").expect("reply accept");
    let protected = forward
        .find("ip daddr @protected4 drop")
        .expect("protected drop");
    assert!(
        spoof < reply,
        "the guest's source is fixed before its answer is admitted"
    );
    assert!(
        reply < protected,
        "the answer goes to the translated client address, which the protected set covers"
    );
    let unpublished = render_with(EgressClass::Denied, Vec::new(), &[]);
    assert!(
        !unpublished.contains("dport 80"),
        "a bundle with no publication names no guest port"
    );
    assert!(
        !unpublished.contains("ct direction reply"),
        "nothing precedes the protected drops when nothing is published"
    );
}

#[test]
fn the_host_table_opens_only_for_traffic_its_publication_table_translated() {
    let names = BundleNames::new("0a0b0c0d");
    let uplink = InterfaceName::new("uplink0").expect("name");
    let protected = ProtectedSet::certified_default();
    let ruleset = HostRuleset {
        names: &names,
        lease: lease(),
        uplink: &uplink,
        zone: ConntrackZone::new(6).expect("zone"),
        protected: &protected,
    }
    .render();
    let forward = chain(&ruleset, "forward");
    let accept = forward
        .find("oifname \"sv0a0b0c0d\" ip daddr 10.200.0.22 ct status dnat accept")
        .expect("the translated connection is admitted");
    let refused = forward
        .find("oifname \"sv0a0b0c0d\" ct state new,invalid,untracked drop")
        .expect("everything else toward the guest is dropped");
    assert!(
        accept < refused,
        "the opening must precede the drop it excepts"
    );
    let input = chain(&ruleset, "input");
    let reply = input
        .find("iifname \"sv0a0b0c0d\" ct status dnat ct state established,related accept")
        .expect("a locally bound publication must receive its reply");
    let closed = input
        .find("iifname \"sv0a0b0c0d\" drop")
        .expect("nothing else may reach the host");
    assert!(reply < closed);
}

#[test]
fn a_ruleset_without_connect_ports_admits_no_connection_out_to_the_guest() {
    let ruleset = render_with(EgressClass::Denied, Vec::new(), &[]);
    assert!(!chain(&ruleset, "output").contains("ct state new"));
    assert!(!chain(&ruleset, "input").contains("ct state"));
}

#[test]
fn connect_ports_admit_only_new_connections_out_to_the_guest_and_their_replies_in() {
    let ruleset = render_connecting(
        EgressClass::Denied,
        Vec::new(),
        &[],
        &[9223..=9224, 80..=80],
    );
    let output = chain(&ruleset, "output");
    assert!(
        output.contains(
            "oifname \"tap0\" ip daddr 10.200.0.22 tcp dport { 9223-9224, 80 } ct state new accept"
        ),
        "{output}"
    );
    // Nothing else in the output chain opens a new connection.
    assert_eq!(output.matches("ct state new").count(), 1, "{output}");
    let input = chain(&ruleset, "input");
    assert!(
        input.contains(
            "iifname \"tap0\" ether saddr 02:01:02:03:04:05 ip saddr 10.200.0.22 ip daddr 10.200.0.21 ct state established,related accept"
        ),
        "{input}"
    );
    // The guest still cannot open a connection toward the gateway: `new` is admitted in no
    // input rule, and the chain keeps its default drop.
    assert!(!input.contains("ct state new"), "{input}");
    assert!(input.contains("policy drop;"));
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
