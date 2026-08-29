//! Shared fixtures: the specification example, a deterministic resolver, ceiling, Backend,
//! and filesystem oracle.

#![allow(dead_code)]

use soma::{OciDigest, OciPlatform};
use soma_template::{
    BackendCapabilities, EgressIntent, IdleAction, IngressIntent, ModuleBuilder, ModuleIdentity,
    ModuleKind, ModuleRegistry, ModuleSpec, PolicyCeiling, Rejection, ResourceLimits, Template,
    TemplateError, TemplateLock, TestFilesystemOracle, TestResolver, parse_template, resolve,
    resolve_with,
};

/// The minimum Template document from `docs/architecture/template-system.md`.
pub const EXAMPLE: &str = r#"schema = "soma.template/v1alpha1"
name = "claude-code-python"

modules = [
  "soma://agent/claude-code@1",
  "soma://tools/git@1",
]

[workload]
image = "python:3.12-slim"
platform = "linux/amd64"

[command]
program = "claude"
args = []
working_directory = "/workspace"

[resources]
vcpus = 2
memory_mib = 2048
writable_storage_mib = 10240

[network]
egress = "deny"
allow_domains = ["api.anthropic.com"]
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 900
maximum_lifetime_seconds = 14400
on_idle = "destroy"

[[environment]]
name = "CI"
value = "true"

[[secrets]]
name = "ANTHROPIC_API_KEY"
source = "secret://anthropic/default"
delivery = "environment"
"#;

pub const PYTHON_DIGEST: &str =
    "sha256:9c1185a5c5e9fc54612808977ee8f548b2258d31ee2c8a2a0e4a7b0d5b2f1c3d";
pub const OTHER_DIGEST: &str =
    "sha256:2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae";
pub const PYTHON_SIZE: u64 = 1_234;

pub fn digest(text: &str) -> OciDigest {
    OciDigest::parse(text).expect("fixture digest is canonical")
}

pub fn amd64() -> OciPlatform {
    OciPlatform::linux_amd64()
}

pub fn arm64() -> OciPlatform {
    OciPlatform::linux_arm64()
}

pub fn resolver() -> TestResolver {
    TestResolver::new()
        .with_image(
            "python:3.12-slim",
            &amd64(),
            digest(PYTHON_DIGEST),
            PYTHON_SIZE,
        )
        .with_image("python:3.12-slim", &arm64(), digest(OTHER_DIGEST), 999)
        .with_image("scratch:static", &amd64(), digest(OTHER_DIGEST), 42)
}

pub fn ceiling() -> PolicyCeiling {
    PolicyCeiling::new(EgressIntent::Allowlist, IngressIntent::Deny)
        .with_domains(&["*.anthropic.com", "api.anthropic.com", "github.com"])
        .expect("bounded ceiling")
        .with_cidrs(&["10.0.0.0/8", "2001:db8::/32"])
        .expect("bounded ceiling")
}

pub fn backend() -> BackendCapabilities {
    BackendCapabilities::new(
        &[amd64(), arm64()],
        &[IdleAction::Destroy, IdleAction::Stop],
        ResourceLimits {
            max_vcpus: 8,
            max_memory_mib: 16_384,
            max_writable_storage_mib: 65_536,
        },
    )
    .expect("bounded backend")
}

pub fn oracle() -> TestFilesystemOracle {
    TestFilesystemOracle::new()
        .with_executable(&digest(PYTHON_DIGEST), "/usr/local/bin/python3")
        .with_executable(&digest(PYTHON_DIGEST), "/bin/sh")
}

pub fn parse(text: &str) -> Template {
    parse_template(text.as_bytes()).expect("fixture parses")
}

pub fn resolve_text(text: &str) -> Result<TemplateLock, TemplateError> {
    resolve(&parse(text), &resolver(), &ceiling(), &backend(), &oracle())
}

pub fn resolve_in(registry: &ModuleRegistry, text: &str) -> Result<TemplateLock, TemplateError> {
    resolve_with(
        registry,
        &parse(text),
        &resolver(),
        &ceiling(),
        &backend(),
        &oracle(),
    )
}

pub fn lock(text: &str) -> TemplateLock {
    resolve_text(text).expect("fixture resolves")
}

pub fn example() -> TemplateLock {
    lock(EXAMPLE)
}

pub fn rejection(result: Result<TemplateLock, TemplateError>) -> Rejection {
    match result {
        Err(TemplateError::Rejected(rejection)) => rejection,
        Ok(_) => panic!("expected a rejection, got a lock"),
        Err(other) => panic!("expected a rejection, got {other:?}"),
    }
}

pub fn reject(text: &str) -> Rejection {
    rejection(resolve_text(text))
}

/// Replaces exactly one occurrence, failing loudly when the anchor is missing.
pub fn edit(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "anchor `{from}` missing from fixture");
    text.replacen(from, to, 1)
}

pub fn module(kind: ModuleKind, name: &str, version: u32) -> ModuleBuilder {
    ModuleSpec::builder(
        ModuleIdentity::new(kind, name, version).expect("valid identity"),
        1,
    )
    .platform(amd64())
    .platform(arm64())
}

pub fn registry(specs: Vec<ModuleSpec>) -> ModuleRegistry {
    let mut registry = ModuleRegistry::builtin();
    for spec in specs {
        registry.register(spec).expect("unique module");
    }
    registry
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        write!(text, "{byte:02x}").expect("String write cannot fail");
        text
    })
}

pub fn unhex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(u8::is_ascii_hexdigit).collect();
    digits
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("ascii");
            u8::from_str_radix(text, 16).expect("hex")
        })
        .collect()
}

/// A minimal valid document over the given modules; `extra` is appended verbatim.
pub fn minimal(modules: &[&str], extra: &str) -> String {
    let list: Vec<String> = modules.iter().map(|m| format!("\"{m}\"")).collect();
    format!(
        "schema = \"soma.template/v1alpha1\"\nname = \"minimal\"\nmodules = [{}]\n\
         [workload]\nimage = \"python:3.12-slim\"\nplatform = \"linux/amd64\"\n\
         [command]\nprogram = \"sh\"\n\
         [resources]\nvcpus = 1\nmemory_mib = 512\nwritable_storage_mib = 1024\n\
         [lifecycle]\nidle_timeout_seconds = 60\nmaximum_lifetime_seconds = 600\non_idle = \"destroy\"\n{extra}",
        list.join(", ")
    )
}

pub fn assert_names(
    rejection: &Rejection,
    class: soma_template::RejectionClass,
    field: &str,
    module: Option<&str>,
) {
    assert_eq!(rejection.class(), class, "{rejection}");
    assert_eq!(rejection.field(), field, "{rejection}");
    assert_eq!(
        rejection.module().map(ToString::to_string).as_deref(),
        module,
        "{rejection}"
    );
    let text = rejection.to_string();
    assert!(text.contains(field), "`{text}` must name field `{field}`");
    if let Some(module) = module {
        assert!(
            text.contains(module),
            "`{text}` must name module `{module}`"
        );
    }
}

/// Replaces the first or last occurrence of `from` with the same-length `to`.
pub fn replace_bytes(bytes: &[u8], from: &str, to: &str, last: bool) -> Vec<u8> {
    assert_eq!(from.len(), to.len(), "substitution must keep the length");
    let positions: Vec<usize> = bytes
        .windows(from.len())
        .enumerate()
        .filter(|(_, window)| *window == from.as_bytes())
        .map(|(index, _)| index)
        .collect();
    assert!(!positions.is_empty(), "`{from}` is encoded");
    let position = if last {
        positions[positions.len() - 1]
    } else {
        positions[0]
    };
    let mut mutated = bytes.to_vec();
    mutated[position..position + to.len()].copy_from_slice(to.as_bytes());
    mutated
}
