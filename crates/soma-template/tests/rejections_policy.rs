//! Rejection classes decided against the operator's ceiling and the module contract: a module
//! slot the Template never fills, a secret reference with no scope, and a network the ceiling
//! does not permit.
//!
//! These are refusals of what the Template asked for, given what it is allowed to ask. The ones
//! decided against what the backend and the resolver can actually do live in
//! `rejections_capability.rs`.

mod support;

use soma_template::{PolicyCeiling, RejectionClass, resolve};
use support::{EXAMPLE, assert_names, backend, edit, oracle, parse, reject, rejection, resolver};

#[test]
fn missing_required_environment_names_the_module_slot() {
    let text = edit(
        EXAMPLE,
        "[[secrets]]\nname = \"ANTHROPIC_API_KEY\"\nsource = \"secret://anthropic/default\"\ndelivery = \"environment\"\n",
        "",
    );
    assert_names(
        &reject(&text),
        RejectionClass::IncompatibleModule,
        "required_environment[0]",
        Some("soma://agent/claude-code@1"),
    );
    let via_launch = edit(
        &text,
        "value = \"true\"",
        "value = \"true\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    );
    let lock = support::lock(&via_launch);
    let required = lock
        .environment()
        .iter()
        .find(|entry| entry.name() == "ANTHROPIC_API_KEY")
        .expect("Launch-required slot is locked");
    assert_eq!(required.value(), None);
}

#[test]
fn secret_reference_without_scope_names_the_secret() {
    for delivery in ["file", "egress-proxy"] {
        let text = edit(
            EXAMPLE,
            "delivery = \"environment\"",
            &format!("delivery = \"{delivery}\""),
        );
        assert_names(
            &reject(&text),
            RejectionClass::SecretWithoutScope,
            "secrets[0].scope",
            None,
        );
    }
    let scoped = edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"file\"\nscope = \"/run/secrets/anthropic\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    );
    let lock = support::lock(&scoped);
    assert_eq!(lock.secrets()[0].scope(), "/run/secrets/anthropic");
    assert_eq!(lock.secrets()[0].mode(), Some(0o400));
    let proxied = edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"egress-proxy\"\nscope = \"api.anthropic.com\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    );
    assert_eq!(
        support::lock(&proxied).secrets()[0].scope(),
        "api.anthropic.com"
    );
    let outside = edit(
        EXAMPLE,
        "delivery = \"environment\"",
        "delivery = \"egress-proxy\"\nscope = \"github.com\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
    );
    assert_names(
        &reject(&outside),
        RejectionClass::InvalidValue,
        "secrets[0].scope",
        None,
    );
}

#[test]
fn network_wider_than_the_ceiling_names_the_widened_field() {
    let unrestricted = edit(
        &edit(EXAMPLE, "allow_domains = [\"api.anthropic.com\"]\n", ""),
        "egress = \"deny\"",
        "egress = \"unrestricted\"",
    );
    assert_names(
        &reject(&unrestricted),
        RejectionClass::NetworkExceedsCeiling,
        "network.egress",
        None,
    );
    let domain = edit(
        EXAMPLE,
        "\"api.anthropic.com\"]",
        "\"api.anthropic.com\", \"evil.example\"]",
    );
    assert_names(
        &reject(&domain),
        RejectionClass::NetworkExceedsCeiling,
        "network.allow_domains[1]",
        None,
    );
    let ingress = edit(EXAMPLE, "ingress = \"deny\"", "ingress = \"unrestricted\"");
    assert_names(
        &reject(&ingress),
        RejectionClass::NetworkExceedsCeiling,
        "network.ingress",
        None,
    );
    let cidr = edit(
        EXAMPLE,
        "ingress = \"deny\"",
        "allow_cidrs = [\"10.1.0.0/16\", \"192.168.0.0/16\"]\ningress = \"deny\"",
    );
    assert_names(
        &reject(&cidr),
        RejectionClass::NetworkExceedsCeiling,
        "network.allow_cidrs[1]",
        None,
    );
    let inside = edit(
        EXAMPLE,
        "ingress = \"deny\"",
        "allow_cidrs = [\"10.1.0.0/16\", \"10.0.0.0/8\", \"10.255.255.255/32\", \"2001:db8:1::/48\"]\ningress = \"deny\"",
    );
    assert!(support::resolve_text(&inside).is_ok());
    for outside in [
        "10.0.0.0/7",
        "11.0.0.0/8",
        "0.0.0.0/0",
        "2001:db9::/32",
        "::ffff:10.0.0.1/128",
        "::/0",
    ] {
        let text = edit(
            EXAMPLE,
            "ingress = \"deny\"",
            &format!("allow_cidrs = [\"{outside}\"]\ningress = \"deny\""),
        );
        assert_names(
            &reject(&text),
            RejectionClass::NetworkExceedsCeiling,
            "network.allow_cidrs[0]",
            None,
        );
    }
    let deny_all = rejection(resolve(
        &parse(EXAMPLE),
        &resolver(),
        &PolicyCeiling::deny_all(),
        &backend(),
        &oracle(),
    ));
    assert_names(
        &deny_all,
        RejectionClass::NetworkExceedsCeiling,
        "network.egress",
        None,
    );
    let wildcard = edit(EXAMPLE, "\"api.anthropic.com\"]", "\"*.anthropic.com\"]");
    assert!(support::resolve_text(&wildcard).is_ok());
    let wider_wildcard = edit(EXAMPLE, "\"api.anthropic.com\"]", "\"*.com\"]");
    assert_eq!(
        reject(&wider_wildcard).class(),
        RejectionClass::NetworkExceedsCeiling
    );
}
