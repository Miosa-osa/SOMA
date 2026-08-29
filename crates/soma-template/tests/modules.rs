//! The module contract, the built-in registry, and the deny-by-default network posture.

mod support;

use soma_template::{
    Command, Destination, EgressEnvelope, EnvironmentName, GuestPath, HealthProbe, IngressIntent,
    ModuleError, ModuleIdentity, ModuleKind, ModuleRef, ModuleRefError, ModuleRegistry, ModuleSpec,
    NameError, PathError, RejectionClass,
};
use support::{EXAMPLE, amd64, assert_names, edit, lock, minimal, module, resolve_in};

#[test]
fn builtin_registry_holds_exactly_the_example_modules() {
    let registry = ModuleRegistry::builtin();
    let identities: Vec<String> = registry.identities().map(ToString::to_string).collect();
    assert_eq!(
        identities,
        [
            "soma://agent/claude-code@1",
            "soma://agent/osa@1",
            "soma://tools/git@1",
            "soma://tools/shell@1",
        ]
    );
    let claude = registry
        .get(&ModuleIdentity::new(ModuleKind::Agent, "claude-code", 1).expect("identity"))
        .expect("registered");
    assert_eq!(claude.schema_version(), 1);
    assert_eq!(
        claude.required_environment()[0].as_str(),
        "ANTHROPIC_API_KEY"
    );
    assert_eq!(claude.secret_environment()[0].as_str(), "ANTHROPIC_API_KEY");
    assert_eq!(claude.destinations()[0].host(), "api.anthropic.com");
    assert_eq!(claude.destinations()[0].port(), 443);
    assert_eq!(
        claude.default_command().map(Command::program),
        Some("claude")
    );
    assert!(claude.platforms().contains(&amd64()));
    let git = registry
        .get(&ModuleIdentity::new(ModuleKind::Tools, "git", 1).expect("identity"))
        .expect("registered");
    assert_eq!(
        git.sealed_environment()[0].0.as_str(),
        "GIT_TERMINAL_PROMPT"
    );
    assert!(git.default_command().is_none());
}

#[test]
fn agent_modules_default_to_denied_egress_and_ingress() {
    let text = minimal(
        &["soma://agent/claude-code@1"],
        "[[secrets]]\nname = \"ANTHROPIC_API_KEY\"\nsource = \"secret://a/b\"\ndelivery = \"environment\"\n",
    );
    let lock = lock(&text);
    assert_eq!(*lock.network().egress(), EgressEnvelope::Deny);
    assert_eq!(lock.network().ingress(), IngressIntent::Deny);
    assert!(
        !lock.network().allows_domain("api.anthropic.com"),
        "a module destination never widens the envelope on its own"
    );
    let text = minimal(&["soma://agent/osa@1"], "");
    let lock = support::lock(&text);
    assert_eq!(*lock.network().egress(), EgressEnvelope::Deny);
    assert_eq!(lock.network().ingress(), IngressIntent::Deny);
}

#[test]
fn module_references_parse_only_the_documented_grammar() {
    let pinned = ModuleRef::parse("soma://agent/claude-code@1").expect("pinned");
    assert_eq!(pinned.kind(), ModuleKind::Agent);
    assert_eq!(pinned.name(), "claude-code");
    assert_eq!(pinned.version(), Some(1));
    assert_eq!(pinned.to_string(), "soma://agent/claude-code@1");
    let unpinned = ModuleRef::parse("soma://tools/git").expect("unpinned");
    assert_eq!(unpinned.version(), None);
    assert!(unpinned.pinned().is_none());
    let cases = [
        ("http://agent/x@1", ModuleRefError::Scheme),
        ("soma://kernel/x@1", ModuleRefError::Kind),
        ("soma://agent/Claude@1", ModuleRefError::Name),
        ("soma://agent/-x@1", ModuleRefError::Name),
        ("soma://agent/x@01", ModuleRefError::Version),
        ("soma://agent/x@", ModuleRefError::Version),
        ("soma://agent/x@4294967296", ModuleRefError::Version),
    ];
    for (text, expected) in cases {
        assert_eq!(ModuleRef::parse(text), Err(expected), "{text}");
    }
    let long = format!("soma://agent/{}@1", "a".repeat(300));
    assert_eq!(ModuleRef::parse(&long), Err(ModuleRefError::TooLong));
}

#[test]
fn registry_and_builder_enforce_their_bounds() {
    let mut registry = ModuleRegistry::empty();
    assert!(registry.is_empty());
    let spec = module(ModuleKind::Tools, "one", 1).build().expect("spec");
    registry.register(spec.clone()).expect("first");
    assert_eq!(registry.len(), 1);
    assert!(matches!(
        registry.register(spec),
        Err(ModuleError::DuplicateModule(_))
    ));
    let identity = ModuleIdentity::new(ModuleKind::Tools, "two", 1).expect("identity");
    assert_eq!(
        ModuleSpec::builder(identity.clone(), 0)
            .platform(amd64())
            .build()
            .err(),
        Some(ModuleError::ZeroSchemaVersion)
    );
    assert_eq!(
        ModuleSpec::builder(identity, 1).build().err(),
        Some(ModuleError::NoPlatform)
    );
    assert!(ModuleIdentity::new(ModuleKind::Tools, "Bad", 1).is_err());
}

#[test]
fn module_digests_are_deterministic_and_content_sensitive() {
    let build = |value: &str| {
        module(ModuleKind::Tools, "digest", 1)
            .sealed_environment(EnvironmentName::parse("MODE").expect("name"), value)
            .build()
            .expect("spec")
            .digest()
    };
    assert_eq!(build("a"), build("a"));
    assert_ne!(build("a"), build("b"));
    let registry = ModuleRegistry::builtin();
    let claude = registry
        .get(&ModuleIdentity::new(ModuleKind::Agent, "claude-code", 1).expect("identity"))
        .expect("registered");
    let locked = lock(EXAMPLE);
    assert_eq!(locked.modules()[0].digest(), &claude.digest());
    assert_eq!(locked.modules()[0].schema_version(), 1);
}

#[test]
fn guest_paths_and_environment_names_are_strict() {
    assert_eq!(GuestPath::parse(""), Err(PathError::Empty));
    assert_eq!(GuestPath::parse("opt"), Err(PathError::NotAbsolute));
    assert_eq!(
        GuestPath::parse("/opt/../etc"),
        Err(PathError::NotNormalized)
    );
    assert_eq!(GuestPath::parse("/opt//bin"), Err(PathError::NotNormalized));
    assert_eq!(GuestPath::parse("/opt/"), Err(PathError::NotNormalized));
    assert_eq!(
        GuestPath::parse("/opt/\n"),
        Err(PathError::ForbiddenCharacter)
    );
    let root = GuestPath::parse("/").expect("root");
    let bin = GuestPath::parse("/opt/bin").expect("bin");
    assert!(root.contains(&bin));
    assert!(GuestPath::parse("/opt").expect("opt").contains(&bin));
    assert!(!GuestPath::parse("/op").expect("op").contains(&bin));
    assert_eq!(bin.file_name(), "bin");
    assert_eq!(root.file_name(), "");
    assert_eq!(EnvironmentName::parse(""), Err(NameError::Empty));
    assert_eq!(
        EnvironmentName::parse("1A"),
        Err(NameError::ForbiddenCharacter)
    );
    assert_eq!(
        EnvironmentName::parse("A-B"),
        Err(NameError::ForbiddenCharacter)
    );
    assert_eq!(
        EnvironmentName::parse(&"A".repeat(257)),
        Err(NameError::TooLong)
    );
    assert!(EnvironmentName::parse("_OK_1").is_ok());
}

#[test]
fn shell_module_provides_the_default_shell_executables() {
    let text = edit(
        EXAMPLE,
        "\"soma://tools/git@1\",",
        "\"soma://tools/shell@1\",",
    );
    let text = edit(&text, "program = \"claude\"", "program = \"bash\"");
    let lock = support::lock(&text);
    assert_eq!(lock.command().program(), "bash");
    assert_eq!(
        lock.modules()[1].identity().to_string(),
        "soma://tools/shell@1"
    );
}

#[test]
fn invalid_module_values_name_the_module_and_field() {
    let port = module(ModuleKind::Tools, "zero-port", 1)
        .destination(Destination::parse("example.com:0").expect("parses"))
        .build()
        .expect("spec");
    let probe = module(ModuleKind::Tools, "zero-probe", 1)
        .health_probe(HealthProbe::Tcp { port: 0 })
        .build()
        .expect("spec");
    let registry = support::registry(vec![port, probe]);
    let text = minimal(&["soma://tools/zero-port@1"], "");
    let rejection = support::rejection(resolve_in(&registry, &text));
    assert_names(
        &rejection,
        RejectionClass::InvalidValue,
        "destinations[0]",
        Some("soma://tools/zero-port@1"),
    );
    let text = minimal(&["soma://tools/zero-probe@1"], "");
    let rejection = support::rejection(resolve_in(&registry, &text));
    assert_names(
        &rejection,
        RejectionClass::InvalidValue,
        "health_probe.port",
        Some("soma://tools/zero-probe@1"),
    );
}
