//! The `soma-template` revision view builds this crate's `TemplateRevision`, and the one
//! place the two contracts disagree is recorded here rather than discovered at compile time.

use soma::{OciDigest, OciPlatform};
use soma_generation::{
    CompileError, CompileErrorKind, LifetimeLimits, StartupBehavior, TemplateImage,
    TemplateRevision,
};
use soma_template::{
    BackendCapabilities, IdleAction, ModuleRegistry, PolicyCeiling, ResourceLimits,
    TemplateRevision as View, TestFilesystemOracle, TestResolver, parse_template, resolve,
};

const DIGEST: &str = "sha256:9c1185a5c5e9fc54612808977ee8f548b2258d31ee2c8a2a0e4a7b0d5b2f1c3d";

/// The specification's resource and lifecycle values over an empty module list, so the
/// fully denied network envelope projects exactly onto the isolated policy.
fn document(vcpus: u32) -> String {
    format!(
        "schema = \"soma.template/v1alpha1\"\nname = \"boundary\"\nmodules = []\n\n\
         [workload]\nimage = \"python:3.12-slim\"\nplatform = \"linux/amd64\"\n\n\
         [command]\nprogram = \"sh\"\n\n\
         [resources]\nvcpus = {vcpus}\nmemory_mib = 2048\nwritable_storage_mib = 10240\n\n\
         [lifecycle]\nidle_timeout_seconds = 900\nmaximum_lifetime_seconds = 14400\non_idle = \"destroy\"\n"
    )
}

/// The same document with `[resources]` and `[lifecycle]` left out, so what reaches the
/// compiler is whatever the schema defaults to rather than what this test chose.
const DEFAULTED: &str = "schema = \"soma.template/v1alpha1\"\nname = \"boundary\"\n\n\
     [workload]\nimage = \"python:3.12-slim\"\nplatform = \"linux/amd64\"\n\n\
     [command]\nprogram = \"sh\"\n";

fn view(vcpus: u32) -> View {
    view_of(&document(vcpus))
}

fn view_of(text: &str) -> View {
    let digest = OciDigest::parse(DIGEST).expect("canonical digest");
    let template = parse_template(text.as_bytes()).expect("document parses");
    let resolver = TestResolver::new().with_image(
        "python:3.12-slim",
        &OciPlatform::linux_amd64(),
        digest.clone(),
        1_234,
    );
    let backend = BackendCapabilities::new(
        &[OciPlatform::linux_amd64()],
        &[IdleAction::Destroy],
        ResourceLimits {
            max_vcpus: 8,
            max_memory_mib: 16_384,
            max_writable_storage_mib: 65_536,
        },
    )
    .expect("bounded backend");
    let oracle = TestFilesystemOracle::new().with_executable(&digest, "/bin/sh");
    let lock = resolve(
        &template,
        &resolver,
        &PolicyCeiling::deny_all(),
        &backend,
        &oracle,
    )
    .expect("document resolves");
    View::from_lock(&lock)
        .with_provenance(&template, &ModuleRegistry::builtin())
        .expect("same document")
}

fn compiler_revision(view: &View) -> Result<TemplateRevision, CompileError> {
    let image = view.image();
    TemplateRevision::new(
        TemplateImage::new(
            image
                .reference()
                .cloned()
                .expect("provenance attached the reference"),
            image.manifest_digest().clone(),
            image.platform().clone(),
        ),
        view.shape().expect("denied envelope projects exactly"),
        StartupBehavior::readiness_only(),
        LifetimeLimits::new(view.ttl_seconds())?,
        1,
    )
}

#[test]
fn the_view_builds_the_compiler_revision_once_the_shape_fits_profile_v1() {
    let two_vcpus = view(2);
    let error = compiler_revision(&two_vcpus).expect_err("profile v1 accepts one vCPU only");
    assert_eq!(error.kind(), CompileErrorKind::Unsupported);
    let one_vcpu = view(1);
    let revision = compiler_revision(&one_vcpu).expect("profile v1 shape");
    assert_eq!(revision.shape().vcpu_count(), 1);
    assert_eq!(revision.shape().memory_mib(), 2048);
    assert_eq!(revision.shape().storage_mib(), 10_240);
    assert_eq!(revision.lifetime().ttl_seconds(), 14_400);
    assert_eq!(revision.image().manifest_digest().as_str(), DIGEST);
    assert_eq!(revision.image().reference().as_str(), "python:3.12-slim");
    assert_eq!(revision.image().platform(), &OciPlatform::linux_amd64());
}

#[test]
fn every_locked_lifetime_is_a_valid_lifetime_limit() {
    let view = view(1);
    assert!(LifetimeLimits::new(view.ttl_seconds()).is_ok());
    let thirty_days = 30 * 24 * 60 * 60;
    assert!(LifetimeLimits::new(thirty_days).is_ok());
    assert!(LifetimeLimits::new(thirty_days + 1).is_err());
}

/// A Template that states no shape and no lifecycle must still compile, because the schema's
/// defaults and the Generation compiler's profile version 1 window have to agree. If profile
/// v1 ever narrows past `MachineShape::DEFAULT_MEMORY_MIB` or `DEFAULT_STORAGE_MIB`, the
/// smallest possible Template stops compiling and this test says so.
#[test]
fn the_schema_defaults_fit_profile_v1_without_being_restated() {
    let revision = compiler_revision(&view_of(DEFAULTED)).expect("defaults are a profile v1 shape");
    assert_eq!(revision.shape().vcpu_count(), 1);
    assert_eq!(revision.shape().memory_mib(), 1_024);
    assert_eq!(revision.shape().storage_mib(), 10_240);
    assert_eq!(revision.lifetime().ttl_seconds(), 3_600);
}
