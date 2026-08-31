//! A Template document on disk resolves against a local OCI layout and becomes compiler input.
//!
//! This is the seam that had nothing on either side of it: `templates/minimal.toml` is resolved
//! with the production resolver and the production filesystem oracle, and the Template Lock that
//! comes out is projected onto the exact `TemplateRevision` the Generation compiler consumes. A
//! full compile needs a kernel and the filesystem tools, so what is asserted here is the
//! Candidate's inputs rather than its bytes.

mod support;

use soma::OciPlatform;
use soma_generation::{
    CompilerProfile, ImportLimits, LayoutResolver, NormalizedRootfs, RootfsOracle,
    compiler_revision, profile_v1_backend,
};
use soma_template::{
    FilesystemOracle, ModuleRegistry, OciResolver, PolicyCeiling, RejectionClass, Template,
    TemplateError, TemplateLock, TemplateRevision as LockRevision, parse_template, resolve,
};
use support::{Fixture, Image, MANIFEST, descriptor, rootfs::TarEntry};

const MINIMAL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../templates/minimal.toml");
const CODING_AGENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/coding-agent.toml"
);

/// A base image shaped like a real Debian one: the programs live under `usr/bin`, and `bin` is a
/// symbolic link to it, so `/bin/sh` only resolves if the oracle follows links.
fn base_layer() -> Vec<u8> {
    support::rootfs::tar(&[
        TarEntry::symlink(b"bin", b"usr/bin"),
        TarEntry::directory(b"usr"),
        TarEntry::directory(b"usr/bin"),
        TarEntry::file(b"usr/bin/sh", b"an executable").mode(0o755),
        TarEntry::file(b"usr/bin/notes", b"not executable").mode(0o644),
    ])
}

struct Prepared {
    fixture: Fixture,
    image: Image,
    normalized: NormalizedRootfs,
}

impl Prepared {
    fn new() -> Self {
        let fixture = Fixture::new();
        let image = support::rootfs::add_layers_for(&fixture, &[base_layer()], "amd64");
        fixture.write_index(&[descriptor(
            MANIFEST,
            &image.manifest_digest,
            image.manifest_size,
        )]);
        let normalized = support::rootfs::normalize_existing(&fixture, &OciPlatform::linux_amd64());
        Self {
            fixture,
            image,
            normalized,
        }
    }

    fn oracle(&self) -> RootfsOracle {
        RootfsOracle::open(
            &self.fixture.store,
            &self.normalized,
            CompilerProfile::v1().tree,
        )
        .expect("the normalized tree decodes")
    }

    fn resolver(&self, template: &Template) -> LayoutResolver {
        LayoutResolver::new(
            &self.fixture.layout,
            template.workload().image(),
            ImportLimits::default(),
        )
    }

    fn resolve(&self, template: &Template) -> Result<TemplateLock, TemplateError> {
        resolve(
            template,
            &self.resolver(template),
            &PolicyCeiling::unrestricted(),
            &profile_v1_backend(),
            &self.oracle(),
        )
    }
}

fn document(path: &str) -> Template {
    let bytes = std::fs::read(path).expect("the shipped template is readable");
    parse_template(&bytes).expect("the shipped template parses")
}

#[test]
fn the_minimal_template_becomes_the_candidate_inputs_the_compiler_binds() {
    let prepared = Prepared::new();
    let template = document(MINIMAL);

    let lock = prepared.resolve(&template).expect("the document resolves");
    let view = LockRevision::from_lock(&lock)
        .with_provenance(&template, &ModuleRegistry::builtin())
        .expect("the same document");
    let revision = compiler_revision(&view, 1).expect("the minimal shape fits profile v1");

    // The digest is the one the layout really holds, not one the test handed to a stub.
    assert_eq!(
        revision.image().manifest_digest().as_str(),
        prepared.image.manifest_digest
    );
    assert_eq!(revision.image().reference().as_str(), "debian:12-slim");
    assert_eq!(revision.image().platform(), &OciPlatform::linux_amd64());
    assert_eq!(revision.shape().vcpu_count(), 1);
    assert_eq!(revision.shape().memory_mib(), 1024);
    assert_eq!(revision.shape().storage_mib(), 2048);
    assert_eq!(revision.lifetime().ttl_seconds(), 900);
    assert_eq!(revision.profile_version(), 1);
    assert_eq!(revision.network_policy(), &soma::NetworkPolicy::isolated());
}

#[test]
fn the_resolved_digest_is_the_manifest_the_layout_holds_for_the_platform() {
    let prepared = Prepared::new();
    let template = document(MINIMAL);

    let resolved = prepared
        .resolver(&template)
        .resolve(template.workload().image(), &OciPlatform::linux_amd64())
        .expect("the layout holds one amd64 image");

    assert_eq!(resolved.digest().as_str(), prepared.image.manifest_digest);
    assert_eq!(
        resolved.size(),
        prepared.image.manifest_size as u64,
        "the resolved size is the manifest's own byte length"
    );
    assert_eq!(resolved.platform(), &OciPlatform::linux_amd64());
}

#[test]
fn the_resolver_refuses_a_reference_the_layout_was_not_exported_for() {
    let prepared = Prepared::new();
    let template = document(MINIMAL);
    let other = soma::OciImage::parse("alpine:3.20").expect("a valid reference");

    let error = prepared
        .resolver(&template)
        .resolve(&other, &OciPlatform::linux_amd64())
        .expect_err("the layout answers for one reference only");

    assert_eq!(error, soma_template::ResolveError::Unresolvable);
}

#[test]
fn the_oracle_follows_the_base_image_symbolic_links_and_the_default_path() {
    let prepared = Prepared::new();
    let template = document(MINIMAL);
    let oracle = prepared.oracle();
    let image = prepared
        .resolver(&template)
        .resolve(template.workload().image(), &OciPlatform::linux_amd64())
        .expect("the layout resolves");
    let present = |program: &str| {
        oracle
            .executable_present(&image, program)
            .expect("the oracle holds this image")
    };

    assert!(present("/bin/sh"), "bin is a link to usr/bin");
    assert!(present("/usr/bin/sh"));
    assert!(present("sh"), "a bare name is looked up along the PATH");
    assert!(
        !present("/usr/bin/notes"),
        "the mode carries no execute bit"
    );
    assert!(!present("/bin/absent"));
    assert!(!present("../bin/sh"), "a relative path starts nowhere");
}

#[test]
fn a_command_the_base_image_lacks_is_rejected_before_any_generation_is_built() {
    let prepared = Prepared::new();
    let mut source = std::fs::read_to_string(MINIMAL).expect("readable");
    source = source.replace("/bin/sh", "/bin/absent");
    let template = parse_template(source.as_bytes()).expect("still parses");

    let error = prepared
        .resolve(&template)
        .expect_err("the program is not in the image");

    let TemplateError::Rejected(rejection) = error else {
        panic!("an absent program is a rejection, not an unavailable dependency");
    };
    assert_eq!(rejection.class(), RejectionClass::ExecutableAbsent);
}

/// The coding-agent Template is the honest limit of this path: it asks for two vCPUs and
/// allowlist egress, and profile v1 admits neither.
#[test]
fn the_coding_agent_template_exceeds_the_compiler_profile() {
    let prepared = Prepared::new();
    let template = document(CODING_AGENT);

    let error = prepared
        .resolve(&template)
        .expect_err("profile v1 admits one vCPU");

    let TemplateError::Rejected(rejection) = error else {
        panic!("a shape above the Backend limit is a rejection");
    };
    assert_eq!(rejection.class(), RejectionClass::InvalidValue);
    assert!(rejection.to_string().contains("resources.vcpus"));
}
