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
    let names = BundleNames::new("0a0b0c0d");
    let intent = intent(egress, resolvers);
    let protected = ProtectedSet::certified_default();
    SandboxRuleset {
        names: &names,
        lease: lease(),
        guest_mac: [0x02, 1, 2, 3, 4, 5],
        intent: &intent,
        protected: &protected,
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
    assert!(forward.contains("udp dport 53 drop") && forward.contains("tcp dport 53 drop"));
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
    let dns_drop = forward.find("udp dport 53 drop").expect("dns drop");
    let egress = forward
        .find("iifname \"tap0\" oifname \"vs0\" ct state new,established accept")
        .expect("egress");
    assert!(dns < dns_drop && dns_drop < egress);
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
