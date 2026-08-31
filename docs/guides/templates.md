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

[resources]
vcpus = 1
memory_mib = 1024
writable_storage_mib = 2048

[lifecycle]
idle_timeout_seconds = 300
maximum_lifetime_seconds = 900
on_idle = "destroy"
```

Then check it:

```console
$ cargo run -p soma-template --example validate -- templates/minimal.toml
name: minimal
image: debian:12-slim
platform: linux/amd64
command: /bin/sh ["-c", "echo hello from soma"]
resources: 1 vcpu, 1024 MiB memory, 2048 MiB writable storage
network: egress Deny, ingress Deny
lock: not computed; pass a pinned image digest to resolve one
```

A Lock identity needs the exact manifest digest of the image, because the Lock records which
bytes were selected and a mutable tag such as `debian:12-slim` does not name any. This example
asks no registry, so pass a digest on the command line:

```console
$ cargo run -p soma-template --example validate -- \
    templates/coding-agent.toml \
    sha256:9c1185a5c5e9fc54612808977ee8f548b2258d31ee2c8a2a0e4a7b0d5b2f1c3d
...
lock: sha256:c82b1d62cc1729e7786854b75955d76eda070a4c8219fa053125ac86dbb186d9
```

That prints a lock for a digest you supplied rather than one anything resolved, which is why it
is a `soma-template` example and not a `soma template` subcommand. To have the digest resolved
for you, and to get a Generation out of the far end, use the build command below.

## Build a Generation from one

`prepare_from_template` runs the whole path: it exports the image to a local OCI layout,
resolves the document against it, and compiles the Template Lock into a prepared-store entry the
KVM backend can launch. It takes the same build inputs as `prepare_generation` (a pinned kernel,
the static guest agent, and the filesystem tools), and it takes the Machine shape, the network
envelope, and the lifetime from the document instead of the command line.

```console
$ skopeo copy --override-os linux --override-arch amd64 \
    docker://docker.io/library/debian:12-slim oci:/tmp/layout:soma
$ cargo run --release -p soma-generation --example prepare_from_template -- \
    templates/minimal.toml \
    /tmp/layout \
    kernel/out/vmlinux-6.12.8-soma-v1 \
    kernel/out/final.config \
    target/x86_64-unknown-linux-musl/release/soma-guest-agent \
    /srv/soma/fs-tools/erofs \
    /srv/soma/fs-tools/e2fsprogs \
    /srv/soma/prepared/minimal
lock: sha256:...
prepared debian:12-slim at /srv/soma/prepared/minimal
  candidate id: ...
  entries: ...
```

Run `scripts/build-soma.sh` and `scripts/build-fs-tools.sh` first for the kernel, the agent, and
the tools. Launch the result by pointing the backend at the parent directory:

```console
$ SOMA_GENERATION_STORE=/srv/soma/prepared SOMA_ALLOW_UNCERTIFIED_GENERATION=1 \
    soma --backend kvm run debian:12-slim -- /bin/sh -c 'echo hello from soma'
```

Two of the document's answers really are resolved here rather than assumed. The image digest
comes from the OCI layout on disk, so the Lock binds the bytes the build is about to compile.
The `command.program` check reads the normalized rootfs the compiler built, following symbolic
links the way the guest will, so `program = "/bin/sh"` is confirmed against Debian's real
`/bin -> usr/bin` layout and a program the image lacks is rejected before anything is built.

### What the Generation does not carry

A Lock binds more than a Generation has room for, and the difference is dropped loudly rather
than quietly. The compiler binds the image, the Machine shape, the readiness behavior, the
lifetime, and the profile version; these locked fields reach nothing:

| Locked field | Why it stops here |
| --- | --- |
| `command` | The Generation has no command field. The workload command arrives at Launch; the locked program is used to check the base image and nothing more. |
| `modules` | Staging module content into the root is ticket T7. A document with modules compiles to the same Generation as one without, so use modules for what they validate, not for what they install. |
| `environment`, `secrets` | Launch-time delivery. Neither the Candidate nor the Generation manifest has a slot for them. |
| `lifecycle.idle_timeout_seconds`, `lifecycle.on_idle` | Only `maximum_lifetime_seconds` projects, as the Generation's time to live. The idle policy belongs to the Backend. |
| `network` with `egress = "allowlist"` or unrestricted ingress | The portable network policy has no destination-filtered egress class, so the projection fails closed rather than widening or narrowing the envelope. |

Two document shapes are therefore refused by this path today even though they are valid
Templates. `templates/coding-agent.toml` is both of them: it asks for two vCPUs, and compiler
profile v1 admits exactly one, and it asks for allowlist egress, which has no portable policy
yet. Compile it and the failure names the field.

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
| `args` | yes | Up to 64 arguments. Write `args = []` when there are none. |
| `working_directory` | no | Absolute path. Defaults to `/`. |
| `user` | no | POSIX user name. Defaults to `root`. |

### `[resources]`

All three are required and all three must be nonzero and within the Backend limits.

| Field | What it does |
| --- | --- |
| `vcpus` | Virtual CPUs given to the Machine. |
| `memory_mib` | Guest memory in MiB. |
| `writable_storage_mib` | Size of the writable overlay in MiB. The base image is read only. |

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

All three are required.

| Field | What it does |
| --- | --- |
| `idle_timeout_seconds` | How long the sandbox may sit idle before `on_idle` fires. |
| `maximum_lifetime_seconds` | Hard ceiling on the sandbox's total life. |
| `on_idle` | `destroy`, `stop`, or `checkpoint`. The Backend must support the action. |

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

## What is verbose about this

The minimum document is twelve required values: `schema`, `name`, `workload.image`,
`workload.platform`, `command.program`, `command.args`, all three of `[resources]`, and all
three of `[lifecycle]`. A Template that runs one command in one image still has to state its
vCPU count, its memory, its writable storage, its idle timeout, its maximum lifetime, and its
idle action, because none of those has a default. The comparable E2B document is
`Template().fromImage("node:24").setStartCmd("npm start")`.

Every one of those values is something SOMA genuinely needs before it can boot a Machine, so
the fix is defaults rather than fewer fields. A default Machine shape and a default lifecycle
would take the minimum document from twelve values to six, and `args = []` could be implied by
its own absence.
