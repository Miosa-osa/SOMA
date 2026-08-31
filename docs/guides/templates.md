# Templates

A Template is a TOML file that says what a sandbox should contain. It selects a base image, a
list of modules, a default command, a Machine shape, a network envelope, and lifecycle limits.
Nothing in it runs. The compiler in `crates/soma-template` turns it into a Template Lock, which
records exactly which inputs were selected and has a content identity of its own.

Working examples live in [`templates/`](../../templates). They are covered by a test, so an
example that stops parsing fails the build.

This guide is the authoring surface. [Creating a Template](creating-templates.md) is the full
reference, including every rejection the compiler can produce.

## Author one

Start from `templates/minimal.toml` and change the image and the command:

```toml
schema = "soma.template/v1alpha1"
name = "minimal"

[workload]
image = "debian:12-slim"
platform = "linux/amd64"

[command]
program = "/bin/sh"
args = ["-c", "echo hello from soma"]
```

Five values have no default and must be written: `schema`, `name`, `workload.image`,
`workload.platform`, and `command.program`. The `args` line above is content rather than
ceremony; a program with no arguments simply leaves it out.

Then check it:

```console
$ cargo run -p soma-template --example validate -- templates/minimal.toml
name: minimal
image: debian:12-slim
platform: linux/amd64
command: /bin/sh ["-c", "echo hello from soma"]
resources: 1 vcpu, 1024 MiB memory, 10240 MiB writable storage
network: egress Deny, ingress Deny
lock: not computed; pass a pinned image digest to resolve one
```

A Lock identity needs the exact manifest digest of the image, because the Lock records which
bytes were selected and a mutable tag such as `debian:12-slim` does not name any. Nothing in
the workspace asks a registry for that digest yet, so pass one on the command line:

```console
$ cargo run -p soma-template --example validate -- \
    templates/coding-agent.toml \
    sha256:9c1185a5c5e9fc54612808977ee8f548b2258d31ee2c8a2a0e4a7b0d5b2f1c3d
...
lock: sha256:c82b1d62cc1729e7786854b75955d76eda070a4c8219fa053125ac86dbb186d9
```

There is no `soma template` subcommand. The example binary stubs the registry lookup and the
image filesystem inspection that a real subcommand would perform, and the `soma` binary should
not print a lock identity it did not really resolve.

## Every field

An unknown field is rejected, so a typo can never silently disable a policy.

### Document

| Field | Required | What it does |
| --- | --- | --- |
| `schema` | yes | Must be `soma.template/v1alpha1`. Checked before anything else. |
| `name` | yes | Up to 128 bytes, no control characters. Names the Template, not the sandbox. |
| `description` | no | Free text. Rejected if it looks like a credential. |
| `modules` | no | Ordered `soma://<kind>/<name>@<version>` references. At most 64. |

### `[workload]`

| Field | Required | What it does |
| --- | --- | --- |
| `image` | yes | An OCI reference. A `@sha256:...` pin is honored and must match what the resolver returns. |
| `platform` | yes | `<os>/<arch>` or `<os>/<arch>/<variant>`. The Backend must support it. |

### `[command]`

Optional as a table. Leave it out only when exactly one module supplies a default command.

| Field | Required | What it does |
| --- | --- | --- |
| `program` | yes | Either a bare name looked up by name or an absolute path. It must exist in the base image or be owned by a module. |
| `args` | no | Up to 64 arguments. Defaults to none, so leave it out when there are none. |
| `working_directory` | no | Absolute path. Defaults to `/`. |
| `user` | no | POSIX user name. Defaults to `root`. |

### `[resources]`

Optional, and so is every field in it. Leaving the table out asks for the same Machine shape
`soma run` asks for when its `--vcpus`, `--memory-mib`, and `--storage-mib` flags are not
given, because both read the same `MachineShape` constants. Stating a field overrides only
that field, so a Template that wants more memory writes `memory_mib` and nothing else.
Whatever the value ends up being, it must be nonzero and within the Backend limits.

| Field | Default | Source | What it does |
| --- | --- | --- | --- |
| `vcpus` | `1` | `MachineShape::DEFAULT_VCPU_COUNT` | Virtual CPUs given to the Machine. |
| `memory_mib` | `1024` | `MachineShape::DEFAULT_MEMORY_MIB` | Guest memory in MiB. |
| `writable_storage_mib` | `10240` | `MachineShape::DEFAULT_STORAGE_MIB` | Size of the writable overlay in MiB. The base image is read only. |

### `[network]`

Optional. Leaving it out denies egress and ingress, which is what a sandbox that talks to
nothing should have.

| Field | What it does |
| --- | --- |
| `egress` | `deny`, `allowlist`, or `unrestricted`. Defaults to `deny`. Never wider than the organization ceiling. |
| `allow_domains` | Domain patterns permitted under `allowlist`. Up to 256. |
| `allow_cidrs` | CIDRs permitted under `allowlist`. Up to 256. |
| `ingress` | `deny` or `unrestricted`. Defaults to `deny`. |

A module that declares a destination needs that destination inside the envelope, so adding
`soma://agent/claude-code@1` means `api.anthropic.com` has to be on the allowlist.

### `[lifecycle]`

Optional, and so is every field in it, on the same terms as `[resources]`. Nothing outside
`crates/soma-template` has a lifecycle opinion to reuse, so the defaults are defined in
`crates/soma-template/src/schema.rs` and that file is the only place they are stated.

| Field | Default | Source | What it does |
| --- | --- | --- | --- |
| `idle_timeout_seconds` | `300` | `DEFAULT_IDLE_TIMEOUT_SECONDS` | How long the sandbox may sit idle before `on_idle` fires. |
| `maximum_lifetime_seconds` | `3600` | `DEFAULT_MAXIMUM_LIFETIME_SECONDS` | Hard ceiling on the sandbox's total life. |
| `on_idle` | `destroy` | `DEFAULT_ON_IDLE` | `destroy`, `stop`, or `checkpoint`. The Backend must support the action. |

Five idle minutes outlasts any interactive round trip but reclaims a sandbox nobody is talking
to in minutes rather than hours. One hour of life bounds the cost of a sandbox someone walked
away from. `destroy` leaves nothing behind, which is the only idle action that is safe to
apply to a Template whose author never thought about idleness; `stop` and `checkpoint` both
retain guest state and are decisions to make on purpose.

### `[[environment]]`

One table per variable. Declare exactly one of `value` or `required = true`; declaring both or
neither is rejected.

| Field | What it does |
| --- | --- |
| `name` | The variable name. |
| `value` | A literal value. Rejected if it looks like a credential. |
| `required` | `true` means Launch must supply it, and a Launch that forgets fails before boot. |

### `[[secrets]]`

A Template holds references to secrets, never values, because a Template is meant to be
committed to a repository.

| Field | Required | What it does |
| --- | --- | --- |
| `name` | yes | How the workload sees the secret. |
| `source` | yes | Where it comes from, such as `secret://anthropic/default`. |
| `delivery` | yes | `environment`, `file`, or `egress-proxy`. |
| `scope` | for `file` and `egress-proxy` | The path or destination the secret is scoped to. Defaults to `name` for `environment`. |
| `mode` | no | File permission bits, for `file` delivery only. Defaults to `0o400`. |

## Worked example

`templates/coding-agent.toml` is the shape most callers want: a language runtime from the base
image, an agent and its tools from modules, and an envelope that names the only two hosts the
sandbox may reach.

```toml
schema = "soma.template/v1alpha1"
name = "coding-agent-python"

modules = [
  "soma://agent/claude-code@1",
  "soma://tools/git@1",
  "soma://tools/shell@1",
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
memory_mib = 4096
writable_storage_mib = 20480

[network]
egress = "allowlist"
allow_domains = ["api.anthropic.com", "github.com"]
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 900
maximum_lifetime_seconds = 14400
on_idle = "destroy"

[[environment]]
name = "CI"
value = "true"

[[environment]]
name = "GIT_AUTHOR_NAME"
required = true

[[secrets]]
name = "ANTHROPIC_API_KEY"
source = "secret://anthropic/default"
delivery = "environment"
```

`program = "claude"` resolves without inspecting the base image because the `claude-code`
module owns `/usr/local/bin/claude`. The same module requires `ANTHROPIC_API_KEY`, which the
`[[secrets]]` table supplies, and declares `api.anthropic.com:443`, which the allowlist admits.
Drop any one of those three and the Template is rejected with the field named.

## What the minimum used to be

The minimum document was twelve values: `schema`, `name`, `workload.image`,
`workload.platform`, `command.program`, `command.args`, all three of `[resources]`, and all
three of `[lifecycle]`. A Template that ran one command in one image still had to state its
vCPU count, its memory, its writable storage, its idle timeout, its maximum lifetime, and its
idle action, because none of those had a default. The comparable E2B document is
`Template().fromImage("node:24").setStartCmd("npm start")`.

Every one of those values is something SOMA genuinely needs before it can boot a Machine, so
the fix was defaults rather than fewer fields, following the pattern `[network]` and
`[command]` already set: an omitted table is a stated intent, not a missing one. `[resources]`
and `[lifecycle]` now default the same way, and `command.args` defaults to no arguments, which
takes the minimum from twelve values to five without removing a single capability. Every value
is still authorable, and a stated value always wins.
