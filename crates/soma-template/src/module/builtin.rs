//! Built-in example modules expressed as data.
//!
//! These are contracts only: what each module owns, needs, and supports.
//! Install recipes belong to the deterministic filesystem construction slice.

use soma::OciPlatform;

use super::{
    Destination, EnvironmentName, GuestPath, HealthProbe, ModuleIdentity, ModuleKind, ModuleRef,
    ModuleRegistry, ModuleSpec,
};
use crate::schema::Command;

struct BuiltinProbe {
    program: &'static str,
    args: &'static [&'static str],
    timeout_seconds: u32,
}

struct BuiltinModule {
    kind: ModuleKind,
    name: &'static str,
    version: u32,
    schema_version: u16,
    requires: &'static [&'static str],
    owned_paths: &'static [&'static str],
    executables: &'static [&'static str],
    required_environment: &'static [&'static str],
    secret_environment: &'static [&'static str],
    sealed_environment: &'static [(&'static str, &'static str)],
    destinations: &'static [&'static str],
    health_probe: Option<BuiltinProbe>,
    platforms: &'static [(&'static str, &'static str)],
    default_command: Option<&'static str>,
}

const LINUX: &[(&str, &str)] = &[("linux", "amd64"), ("linux", "arm64")];

const MODULES: &[BuiltinModule] = &[
    BuiltinModule {
        kind: ModuleKind::Agent,
        name: "claude-code",
        version: 1,
        schema_version: 1,
        requires: &[],
        owned_paths: &[
            "/usr/local/bin/claude",
            "/usr/local/lib/soma/agents/claude-code",
        ],
        executables: &["/usr/local/bin/claude"],
        required_environment: &["ANTHROPIC_API_KEY"],
        secret_environment: &["ANTHROPIC_API_KEY"],
        sealed_environment: &[],
        destinations: &["api.anthropic.com:443"],
        health_probe: Some(BuiltinProbe {
            program: "/usr/local/bin/claude",
            args: &["--version"],
            timeout_seconds: 30,
        }),
        platforms: LINUX,
        default_command: Some("claude"),
    },
    BuiltinModule {
        kind: ModuleKind::Agent,
        name: "osa",
        version: 1,
        schema_version: 1,
        requires: &[],
        owned_paths: &["/usr/local/bin/osa", "/usr/local/lib/soma/agents/osa"],
        executables: &["/usr/local/bin/osa"],
        required_environment: &[],
        secret_environment: &["OSA_API_KEY"],
        sealed_environment: &[],
        destinations: &[],
        health_probe: Some(BuiltinProbe {
            program: "/usr/local/bin/osa",
            args: &["--version"],
            timeout_seconds: 30,
        }),
        platforms: LINUX,
        default_command: Some("osa"),
    },
    BuiltinModule {
        kind: ModuleKind::Tools,
        name: "git",
        version: 1,
        schema_version: 1,
        requires: &[],
        owned_paths: &["/usr/bin/git", "/usr/lib/git-core"],
        executables: &["/usr/bin/git"],
        required_environment: &[],
        secret_environment: &[],
        sealed_environment: &[("GIT_TERMINAL_PROMPT", "0")],
        destinations: &[],
        health_probe: Some(BuiltinProbe {
            program: "/usr/bin/git",
            args: &["--version"],
            timeout_seconds: 10,
        }),
        platforms: LINUX,
        default_command: None,
    },
    BuiltinModule {
        kind: ModuleKind::Tools,
        name: "shell",
        version: 1,
        schema_version: 1,
        requires: &[],
        owned_paths: &["/usr/local/lib/soma/tools/shell"],
        executables: &["/bin/sh", "/bin/bash"],
        required_environment: &[],
        secret_environment: &[],
        sealed_environment: &[],
        destinations: &[],
        health_probe: Some(BuiltinProbe {
            program: "/bin/sh",
            args: &["-c", "true"],
            timeout_seconds: 10,
        }),
        platforms: LINUX,
        default_command: None,
    },
];

/// Every built-in module in one registry; the data is static and checked by tests.
pub(super) fn registry() -> ModuleRegistry {
    let mut registry = ModuleRegistry::empty();
    for spec in MODULES.iter().map(build) {
        registry
            .register(spec)
            .expect("built-in module data is bounded and unique");
    }
    registry
}

fn build(data: &BuiltinModule) -> ModuleSpec {
    const VALID: &str = "built-in module data is valid";
    let identity = ModuleIdentity::new(data.kind, data.name, data.version).expect(VALID);
    let mut builder = ModuleSpec::builder(identity, data.schema_version);
    for reference in data.requires {
        builder = builder.requires(ModuleRef::parse(reference).expect(VALID));
    }
    for path in data.owned_paths {
        builder = builder.owned_path(GuestPath::parse(path).expect(VALID));
    }
    for path in data.executables {
        builder = builder.executable(GuestPath::parse(path).expect(VALID));
    }
    for name in data.required_environment {
        builder = builder.required_environment(EnvironmentName::parse(name).expect(VALID));
    }
    for name in data.secret_environment {
        builder = builder.secret_environment(EnvironmentName::parse(name).expect(VALID));
    }
    for (name, value) in data.sealed_environment {
        builder = builder.sealed_environment(EnvironmentName::parse(name).expect(VALID), value);
    }
    for destination in data.destinations {
        builder = builder.destination(Destination::parse(destination).expect(VALID));
    }
    if let Some(probe) = &data.health_probe {
        builder = builder.health_probe(HealthProbe::Command {
            program: probe.program.to_owned(),
            args: probe.args.iter().map(|arg| (*arg).to_owned()).collect(),
            timeout_seconds: probe.timeout_seconds,
        });
    }
    for (operating_system, architecture) in data.platforms {
        builder = builder
            .platform(OciPlatform::new(*operating_system, *architecture, None).expect(VALID));
    }
    if let Some(program) = data.default_command {
        builder = builder.default_command(Command::new(program, &[]).expect(VALID));
    }
    builder.build().expect(VALID)
}
