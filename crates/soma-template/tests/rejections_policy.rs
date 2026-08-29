//! Rejection classes decided against external inputs: image resolution, platform and module
//! compatibility, secret scope, the policy ceiling, and Backend lifecycle support.

mod support;

use soma::{OciImage, OciPlatform};
use soma_template::{
    BackendCapabilities, ExternalDependency, IdleAction, OciResolver, PolicyCeiling,
    RejectionClass, ResolveError, ResolvedImage, ResourceLimits, TemplateError, TestResolver,
    resolve,
};
use support::{
    EXAMPLE, OTHER_DIGEST, PYTHON_DIGEST, amd64, assert_names, backend, ceiling, digest, edit,
    minimal, module, oracle, parse, registry, reject, rejection, resolve_in, resolver,
};

#[test]
fn unresolvable_image_names_the_workload_field() {
    let unknown = reject(&edit(EXAMPLE, "python:3.12-slim", "python:3.13-slim"));
    assert_names(
        &unknown,
        RejectionClass::UnresolvableImage,
        "workload.image",
        None,
    );
    assert!(unknown.to_string().contains("python:3.13-slim"));
    let pinned = format!("python@{OTHER_DIGEST}");
    let lying = TestResolver::new().with_image(&pinned, &amd64(), digest(PYTHON_DIGEST), 1);
    let template = parse(&edit(EXAMPLE, "python:3.12-slim", &pinned));
    let mismatch = rejection(resolve(
        &template,
        &lying,
        &ceiling(),
        &backend(),
        &oracle(),
    ));
    assert_names(
        &mismatch,
        RejectionClass::UnresolvableImage,
        "workload.image",
        None,
    );
    let honest = TestResolver::new().with_image(&pinned, &amd64(), digest(OTHER_DIGEST), 1);
    let lock = resolve(&template, &honest, &ceiling(), &backend(), &oracle()).expect("pinned");
    assert_eq!(lock.image().digest().as_str(), OTHER_DIGEST);
}

#[test]
fn resolver_outage_is_not_a_rejection() {
    struct Down;
    impl OciResolver for Down {
        fn resolve(&self, _: &OciImage, _: &OciPlatform) -> Result<ResolvedImage, ResolveError> {
            Err(ResolveError::Unavailable("registry offline".to_owned()))
        }
    }
    let error =
        resolve(&parse(EXAMPLE), &Down, &ceiling(), &backend(), &oracle()).expect_err("outage");
    assert!(matches!(
        error,
        TemplateError::Unavailable {
            dependency: ExternalDependency::OciResolver,
            ..
        }
    ));
    assert!(error.rejection().is_none());
}

#[test]
fn unsupported_platform_names_backend_or_module() {
    let amd64_only =
        BackendCapabilities::new(&[amd64()], &[IdleAction::Destroy], backend().limits())
            .expect("bounded");
    let arm = parse(&edit(EXAMPLE, "linux/amd64", "linux/arm64"));
    let backend_rejects = rejection(resolve(
        &arm,
        &resolver(),
        &ceiling(),
        &amd64_only,
        &oracle(),
    ));
    assert_names(
        &backend_rejects,
        RejectionClass::IncompatibleModule,
        "workload.platform",
        None,
    );
    let narrow = module(soma_template::ModuleKind::Tools, "amd-only", 1);
    let narrow =
        soma_template::ModuleSpec::builder(narrow.build().expect("spec").identity().clone(), 1)
            .platform(amd64())
            .build()
            .expect("spec");
    let text = edit(
        &edit(EXAMPLE, "linux/amd64", "linux/arm64"),
        "\"soma://tools/git@1\",",
        "\"soma://tools/amd-only@1\",",
    );
    let module_rejects = rejection(resolve_in(&registry(vec![narrow]), &text));
    assert_names(
        &module_rejects,
        RejectionClass::IncompatibleModule,
        "platforms",
        Some("soma://tools/amd-only@1"),
    );
}

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

#[test]
fn lifecycle_action_unsupported_by_the_backend_names_on_idle() {
    let text = edit(EXAMPLE, "on_idle = \"destroy\"", "on_idle = \"checkpoint\"");
    let rejection = reject(&text);
    assert_names(
        &rejection,
        RejectionClass::UnsupportedLifecycleAction,
        "lifecycle.on_idle",
        None,
    );
    assert!(rejection.to_string().contains("checkpoint"));
    let stop = edit(EXAMPLE, "on_idle = \"destroy\"", "on_idle = \"stop\"");
    assert!(support::resolve_text(&stop).is_ok());
    let limits = ResourceLimits {
        max_vcpus: 1,
        max_memory_mib: 512,
        max_writable_storage_mib: 1024,
    };
    let checkpointing =
        BackendCapabilities::new(&[amd64()], &[IdleAction::Checkpoint], limits).expect("bounded");
    let text = minimal(&[], "");
    let text = edit(&text, "on_idle = \"destroy\"", "on_idle = \"checkpoint\"");
    let lock = resolve(
        &parse(&text),
        &resolver(),
        &ceiling(),
        &checkpointing,
        &oracle(),
    );
    assert!(lock.is_ok());
}
