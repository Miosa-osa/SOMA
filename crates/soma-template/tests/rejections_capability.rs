//! Rejection classes decided against what the backend and the resolver can actually do: an
//! image no registry resolves or that resolves to something else, a platform neither the
//! backend nor a module supports, and an idle action the backend cannot perform.
//!
//! A resolver that is merely down is proved here too, because it is the one case that must not
//! become a rejection: an outage says nothing about whether the Template is admissible. The
//! refusals decided against the operator's ceiling live in `rejections_policy.rs`.

mod support;

use soma::{OciImage, OciPlatform};
use soma_template::{
    BackendCapabilities, ExternalDependency, IdleAction, OciResolver, RejectionClass, ResolveError,
    ResolvedImage, ResourceLimits, TemplateError, TestResolver, resolve,
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
