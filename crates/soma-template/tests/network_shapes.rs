//! Domain and CIDR shape rules: what an allowlist entry may look like before the ceiling is
//! consulted.

mod support;

use soma_template::{
    BoundError, Destination, EgressEnvelope, EgressIntent, IngressIntent, InvalidReason, LockError,
    ModuleKind, PolicyCeiling, Rejection, RejectionClass, TemplateLock, resolve,
};
use support::{
    EXAMPLE, assert_names, backend, edit, lock, minimal, module, oracle, parse, registry, reject,
    rejection, replace_bytes, resolve_in, resolver,
};

fn with_cidrs(list: &str) -> String {
    edit(
        EXAMPLE,
        "ingress = \"deny\"",
        &format!("allow_cidrs = [{list}]\ningress = \"deny\""),
    )
}

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

#[test]
fn cidrs_are_locked_in_canonical_text() {
    let canonical = lock(&with_cidrs("\"10.0.0.0/8\", \"2001:db8::/32\""));
    let variants = lock(&with_cidrs(
        "\"2001:DB8:0000:0000::/32\", \"10.0.0.0/8\", \"10.0.0.0/8\", \"2001:db8::/32\"",
    ));
    assert_eq!(canonical.id(), variants.id());
    let EgressEnvelope::Allowlist { cidrs, .. } = variants.network().egress() else {
        panic!("destinations form an allowlist envelope");
    };
    assert_eq!(cidrs, &["10.0.0.0/8", "2001:db8::/32"]);
    for host_bits in [
        "10.255.255.255/8",
        "10.0.0.1/24",
        "2001:db8::1/32",
        "10.0.0.0/08",
        "10.0.0.0/33",
        "2001:db8::/129",
        "10.0.0.0",
        "10.0.0.0/",
    ] {
        let rejection = reject(&with_cidrs(&format!("\"{host_bits}\"")));
        assert_names(
            &rejection,
            RejectionClass::InvalidValue,
            "network.allow_cidrs[0]",
            None,
        );
        assert!(
            matches!(
                rejection,
                Rejection::InvalidValue {
                    reason: InvalidReason::InvalidCidr,
                    ..
                }
            ),
            "{host_bits}"
        );
    }
}

#[test]
fn ceilings_normalize_cidrs_and_check_destination_shapes() {
    let ceiling = PolicyCeiling::new(EgressIntent::Allowlist, IngressIntent::Deny)
        .with_cidrs(&["2001:DB8::/32", "10.0.0.0/8", "10.0.0.0/8"])
        .expect("canonical");
    assert_eq!(
        ceiling.cidrs(),
        Some(&["10.0.0.0/8".to_owned(), "2001:db8::/32".to_owned()][..])
    );
    let text = with_cidrs("\"2001:0db8::/32\"");
    assert!(resolve(&parse(&text), &resolver(), &ceiling, &backend(), &oracle()).is_ok());
    for (cidrs, field) in [
        (&["10.0.0.0/8", "10.0.0.1/8"][..], "ceiling.cidrs[1]"),
        (&["10.0.0.0"][..], "ceiling.cidrs[0]"),
    ] {
        let error = PolicyCeiling::unrestricted()
            .with_cidrs(cidrs)
            .expect_err("host bits or shape");
        assert_eq!(
            error,
            BoundError::InvalidShape {
                field: field.to_owned()
            }
        );
    }
    let error = PolicyCeiling::unrestricted()
        .with_domains(&["api.anthropic.com", "10.0.0.1"])
        .expect_err("literal");
    assert_eq!(
        error,
        BoundError::InvalidShape {
            field: "ceiling.domains[1]".to_owned()
        }
    );
}

#[test]
fn the_decoder_accepts_only_canonical_cidrs_in_the_envelope_and_the_ceiling() {
    let bytes = lock(&with_cidrs("\"10.0.0.0/8\"")).encode();
    assert!(TemplateLock::decode(&bytes).is_ok());
    assert_eq!(
        TemplateLock::decode(&replace_bytes(&bytes, "10.0.0.0/8", "10.0.0.1/8", false)),
        Err(LockError::InvalidField {
            field: "network.egress"
        })
    );
    assert_eq!(
        TemplateLock::decode(&replace_bytes(&bytes, "10.0.0.0/8", "10.0.0.1/8", true)),
        Err(LockError::InvalidField {
            field: "ceiling.list"
        })
    );
    assert_eq!(
        TemplateLock::decode(&replace_bytes(
            &bytes,
            "2001:db8::/32",
            "2001:DB8::/32",
            true
        )),
        Err(LockError::InvalidField {
            field: "ceiling.list"
        })
    );
    assert_eq!(
        TemplateLock::decode(&replace_bytes(
            &bytes,
            "api.anthropic.com",
            "api.anthropic.123",
            false
        )),
        Err(LockError::InvalidField {
            field: "network.egress"
        })
    );
}
