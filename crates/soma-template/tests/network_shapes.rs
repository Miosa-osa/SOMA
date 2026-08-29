//! Domain and CIDR shape rules: what an allowlist entry may look like before the ceiling is
//! consulted.

mod support;

use soma_template::{
    Destination, EgressIntent, IngressIntent, InvalidReason, ModuleKind, PolicyCeiling, Rejection,
    RejectionClass, resolve,
};
use support::{
    EXAMPLE, assert_names, backend, edit, minimal, module, oracle, parse, registry, rejection,
    resolve_in, resolver,
};

fn with_domains(list: &str) -> String {
    edit(
        EXAMPLE,
        "allow_domains = [\"api.anthropic.com\"]",
        &format!("allow_domains = [{list}]"),
    )
}

fn invalid_domain(rejection: &Rejection, field: &str, module: Option<&str>) {
    assert_names(rejection, RejectionClass::InvalidValue, field, module);
    assert!(matches!(
        rejection,
        Rejection::InvalidValue {
            reason: InvalidReason::InvalidDomain,
            ..
        }
    ));
}

#[test]
fn an_ip_literal_is_not_a_domain_and_cannot_bypass_a_cidr_ceiling() {
    let cidr_only = PolicyCeiling::new(EgressIntent::Allowlist, IngressIntent::Deny)
        .with_cidrs(&["10.0.0.0/8"])
        .expect("bounded");
    for literal in ["169.254.169.254", "10.0.0.1", "*.10.0.0", "example.123"] {
        let text = with_domains(&format!("\"{literal}\""));
        let rejected = rejection(resolve(
            &parse(&text),
            &resolver(),
            &cidr_only,
            &backend(),
            &oracle(),
        ));
        invalid_domain(&rejected, "network.allow_domains[0]", None);
    }
    let open = PolicyCeiling::unrestricted();
    for accepted in [
        "1example.com",
        "a1.example",
        "localhost",
        "*.svc.cluster.local",
    ] {
        let text = with_domains(&format!("\"{accepted}\""));
        assert!(
            resolve(&parse(&text), &resolver(), &open, &backend(), &oracle()).is_ok(),
            "{accepted}"
        );
    }
}

#[test]
fn ip_literals_are_rejected_as_module_destinations_and_proxy_scopes() {
    let numeric = module(ModuleKind::Tools, "numeric", 1)
        .destination(Destination::parse("10.0.0.1:443").expect("parses"))
        .build()
        .expect("spec");
    let registry = registry(vec![numeric]);
    let rejected = rejection(resolve_in(
        &registry,
        &minimal(&["soma://tools/numeric@1"], ""),
    ));
    invalid_domain(&rejected, "destinations[0]", Some("soma://tools/numeric@1"));
    let text = edit(
        &edit(
            EXAMPLE,
            "delivery = \"environment\"",
            "delivery = \"egress-proxy\"\nscope = \"169.254.169.254\"\n\n[[environment]]\nname = \"ANTHROPIC_API_KEY\"\nrequired = true",
        ),
        "egress = \"deny\"\nallow_domains = [\"api.anthropic.com\"]",
        "egress = \"unrestricted\"",
    );
    let rejected = rejection(resolve(
        &parse(&text),
        &resolver(),
        &PolicyCeiling::unrestricted(),
        &backend(),
        &oracle(),
    ));
    invalid_domain(&rejected, "secrets[0].scope", None);
}
