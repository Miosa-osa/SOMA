# Creating a SOMA Template

This guide explains how to author a SOMA Template when you want a sandbox with specific things inside it.
It describes what the `soma.template/v1alpha1` schema accepts, what the compiler in `crates/soma-template` enforces at this revision, and what happens to a Template after it compiles.
The design behind it is [the template system](../architecture/template-system.md) and [ADR 0022](../adr/0022-compose-templates-into-generation-locks.md).

Read the status note in section 7 before planning around this guide.
Status words here are the five terms defined in [the engineering standard](../standards/sota-engineering-standard.md#status-vocabulary): designed, component-tested, live-proved, integrated, production-admitted, and [the claim ledger](../claim-ledger.md) carries them in one table.
At this revision the Template compiler is component-tested as a Rust library, no `soma` command or MCP tool consumes a Template document, and the KVM Launch of a compiled Generation is live-proved only by an ignored test on a Linux x86_64 host.
Every document and error text in this guide was checked against `crates/soma-template` at the revision that added this file.

## 1. What a Template is and is not

A Template is a reusable user-authored recipe.
It selects one OCI image, an ordered list of modules, a default command, a Machine shape, a network envelope, lifecycle limits, environment slots, and secret references.
It is a TOML file that you can commit, review, and reuse.

A Template is not a running sandbox.
It is not a Snapshot, and it is not an image.
Nothing in a Template runs until it has been compiled into a lock and then built into a Generation.

```text
Template            what should be prepared        editable TOML
    |
    | parse, compose, resolve, validate
    v
Template Lock       exactly which inputs were selected     SOMALOCK bytes, LockId
    |
    | build and certify, outside Launch
    v
Generation          verified immutable artifacts   EROFS root, kernel, initramfs, manifest, GenerationId
    |
    | Launch with fresh state
    v
Instance            one running sandbox            fresh identity, private RAM, private disk
```

The compiler turns the Template into a Template Lock.
The lock records the exact OCI manifest digest, the composed module identities and digests, the effective command with defaults applied, the resources, the normalized network envelope, the lifecycle, the environment contract, the secret references, the policy ceiling, and the Backend capabilities.
Two documents that select the same things produce the same lock bytes and the same `LockId`, whatever their spelling or field order.

The Generation compiler in `crates/soma-generation` then builds immutable artifacts from the lock and the normalized image tree.
A Generation never runs itself.

Launch realizes a Generation as an Instance.
Every Launch creates a fresh Instance with its own identity, memory, writable disk, network identity, and guest authority.
Launching the same Generation twice creates two different Instances that share only immutable bytes.
Editing a Template does not change any existing Instance; it produces a new lock, normally a new Generation, and new Instances from then on.

## 2. The smallest Template and what you get inside it

The smallest valid document names the schema, the Template, one image and platform, one command, three resource dimensions, and three lifecycle values.

```toml
schema = "soma.template/v1alpha1"
name = "node-version"

[workload]
image = "node:22"
platform = "linux/amd64"

[command]
program = "/usr/local/bin/node"
args = ["--version"]

[resources]
vcpus = 1
memory_mib = 1024
writable_storage_mib = 1024

[lifecycle]
idle_timeout_seconds = 300
maximum_lifetime_seconds = 3600
on_idle = "destroy"
```

`[network]` may be omitted and then means denied egress and denied ingress.
`[command]` may be omitted only when exactly one composed module supplies a default command, and this document composes no modules.
`modules`, `[[environment]]`, and `[[secrets]]` are optional.

### Where the files inside come from

The guest filesystem is the selected OCI image and almost nothing else.
[Visual atlas section 4](../architecture/visual-atlas.md#4-where-the-first-file-and-first-directory-come-from) shows the host-side Generation artifacts and the guest view of `/`.
[Visual atlas section 5](../architecture/visual-atlas.md#5-where-node-python-or-another-runtime-comes-from) explains that a Workload runtime such as Node or Python exists only because the selected image contains it.

The Generation compiler normalizes the image layers into one logical tree and formats that tree into an immutable EROFS root.
For the `node:22` revision cached on the Linux development host, that tree had 33,512 entries and the EROFS root was 1,129,172,992 bytes.
`/usr/local/bin/node` exists inside the sandbox because that path is in the image, and `/usr/local/bin/node --version` returned `v22.23.2` on a real cold boot of that Generation, as recorded in [the first sandbox command evidence](../evidence/2026-08-29-x86_64-first-sandbox-command.md).

SOMA adds the following, and only the following, on top of the image.

| Added by SOMA | Where it comes from | Visible as a file in the image? |
|---|---|---|
| The Guest agent, PID 1 | The Generation initramfs, which carries `/init` and `/bin/soma-guest-agent`; the responder secret reaches the guest only through the non-snapshot launch page | No; after the root switch the agent keeps running from memory and the initramfs is gone |
| `/dev`, `/proc`, `/sys` | Mounted by the Guest agent during early init and moved into the composed root | Directories created on the private overlay if the image lacks them |
| `/run` and `/tmp` | Fresh tmpfs mounts of 16 MiB and 64 MiB made during identity Repair | Directories created on the private overlay if the image lacks them |
| `/etc/hostname`, `/etc/machine-id`, `/etc/hosts`, `/etc/resolv.conf` | Written during identity and network Repair for this Instance | Written to the private overlay; the image bytes are untouched |
| The writable root | One Instance-private ext4 overlay head over the read-only EROFS root | No; the head is a separate block device |

The hostname is `soma-` plus twelve hex characters derived from the Instance identity, so `uname -a` inside the busybox run printed `soma-4021a60cea3a`.

### What is not there

There is no shell unless the image contains one.
`/bin/sh -c "..."` in an image without `/bin/sh` fails with executable-not-found, exactly as the table in [visual atlas section 5](../architecture/visual-atlas.md#what-happens-when-the-image-is-wrong) states.
There is no package manager step, no `apt-get`, no `npm install`, and no download during Launch.
Nothing from the image is started on its own: no init system, no service manager, and no daemon, and the image's own `ENTRYPOINT` and `CMD` are not consulted by the Template compiler.

### Commands are argument vectors

Every command in SOMA is an executable path plus an argument array.
`program = "/usr/local/bin/node"` with `args = ["--version"]` becomes one `execve` inside the guest without a host shell or a guest shell.
If you need shell behavior, name the shell as the program, for example `program = "/bin/sh"` with `args = ["-c", "echo hello"]`, and make sure the image has that shell.

At this revision the Guest agent runs every command as root, with `/` as the working directory, with standard input closed, and with this fixed environment: `HOME=/root`, `LANG=C.UTF-8`, `PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`, `SOMA_SANDBOX=1`, and `TERM=dumb`.
The application wire contract version 1 carries no environment, working-directory, or input fields.
The Template compiler locks `working_directory`, `user`, `[[environment]]`, and `[[secrets]]`, but no code delivers them into a guest yet; see section 7.

## 3. The `soma.template/v1alpha1` schema field by field

The document is TOML, at most 256 KiB, and UTF-8.
Every string is at most 4,096 bytes and may not contain a NUL byte.
Unknown keys are rejected during parsing with their full dotted path, for example `` unknown field `workload.tag` ``, so a misspelled policy key can never silently disable a policy.
Within one table a missing or mistyped required field is reported before an unknown key, and an unsupported `schema` value is reported before anything else.

The parser checks shape and bounds.
The validator then checks semantics in one fixed order: platforms, resources, lifecycle, description, command shape, module values, environment, network envelope, secrets, required environment, and finally the executable check.
The validator needs four external inputs, all of which are seams with deterministic test implementations at this revision: an OCI resolver, a policy ceiling, the Backend capabilities, and a filesystem oracle.

### Top-level keys

| Key | Required | Accepted | Notes |
|---|---|---|---|
| `schema` | Yes | Exactly `soma.template/v1alpha1` | Checked before any other key |
| `name` | Yes | 1 to 128 bytes, no ASCII control characters | Never enters the lock, so renaming keeps the `LockId` |
| `description` | No | Up to 4,096 bytes | Never enters the lock; rejected if it carries a credential-shaped literal |
| `modules` | No | Array of up to 64 `soma://<kind>/<name>@<version>` strings | Must appear before the first `[table]` header, because TOML assigns later keys to the most recent table |

A module reference is `soma://<kind>/<name>@<version>`.
The kind is one of `agent`, `tools`, `workspace`, `network`, `environment`, `secrets`, `lifecycle`, or `resources`.
The name is lowercase ASCII letters, digits, and interior hyphens, at most 64 bytes.
The version is a decimal `u32` without leading zeros, and a reference without `@<version>` is rejected as unpinned.
The whole reference is at most 256 bytes.

### `[workload]`

| Key | Required | Accepted | Notes |
|---|---|---|---|
| `image` | Yes | Up to 1,024 bytes, no `://`, no whitespace, no backslash, not starting with `-` | A tag such as `python:3.12-slim` is an authoring convenience; resolution pins the manifest digest |
| `platform` | Yes | `<os>/<architecture>` or `<os>/<architecture>/<variant>` | Must be supported by the Backend and by every composed module |

An image written as `name@sha256:<digest>` is checked against the resolver's answer, and a resolver that returns a different digest is a rejection.

### `[command]`

| Key | Required | Accepted | Default |
|---|---|---|---|
| `program` | Yes | 1 to 4,096 bytes, no control characters; an absolute guest path or a bare name | none |
| `args` | No | Up to 64 strings | `[]` |
| `working_directory` | No | An absolute, normalized guest path with no `.` or `..` segments and no trailing slash | `/` |
| `user` | No | A lowercase letter or underscore, then lowercase letters, digits, `_`, or `-`, at most 32 bytes | `root` |

The whole table may be omitted.
Composition then requires exactly one default command among the composed modules, and two modules that disagree are a rejection.
The executable check accepts a bare name when a composed module exports an executable with that file name or when the filesystem oracle finds it by name, and accepts an absolute path when a module exports exactly that path or the oracle finds it.
A relative path such as `bin/claude` is looked up as an exact path and fails.
A bare name passes the executable check but cannot be executed: the application wire contract version 1 rejects any program that does not begin with `/`, so a bare-name command, including a module default command such as `claude`, is unrunnable until a build plan installs the program and the wire contract carries a resolved absolute path.
Nothing checks that `working_directory` exists or that `user` exists in the image's `/etc/passwd`.

### `[resources]`

| Key | Required | Accepted |
|---|---|---|
| `vcpus` | Yes | Nonzero, at most the Backend maximum, and at most 65,535 |
| `memory_mib` | Yes | Nonzero and at most the Backend maximum |
| `writable_storage_mib` | Yes | Nonzero and at most the Backend maximum |

The document carries no defaults; all three dimensions must be written.
The Backend maximums are inputs to validation and are bound into the lock.
Process count, open file, and output limits from the design are not accepted by this schema version; the parser rejects those keys as unknown.

The Generation compiler profile version 1 is narrower than the validator: it accepts exactly one vCPU, memory from 128 MiB through 3 GiB, and writable storage of at least 64 MiB in 4 MiB units, on `linux/amd64` only.
A Template with `vcpus = 2` compiles to a valid lock that the Generation compiler then rejects as unsupported; `crates/soma-generation/tests/template_boundary.rs` records that mismatch.

### `[network]`

| Key | Required | Accepted | Default |
|---|---|---|---|
| `egress` | No | `deny`, `allowlist`, or `unrestricted` | `deny` |
| `allow_domains` | No | Up to 256 lowercase DNS names, optionally prefixed with `*.`, at most 253 bytes with labels of at most 63 bytes, final label not all digits | `[]` |
| `allow_cidrs` | No | Up to 256 `address/prefix` values with every host bit clear and no leading zeros in the prefix | `[]` |
| `ingress` | No | `deny` or `unrestricted` | `deny` |

The whole table may be omitted and then means denied egress and denied ingress.
`deny` with destinations listed and `allowlist` with the same destinations are one envelope and produce one `LockId`.
`allowlist` with no destination is a rejection, and `unrestricted` with any destination is a rejection.
An IPv4 literal such as `169.254.169.254` is not a domain; declare an address under `allow_cidrs` with an explicit prefix.
Destinations are stored sorted and deduplicated, and every CIDR is stored in one canonical text, so `2001:DB8::/32` and `2001:db8::/32` are the same entry.

The envelope is a maximum, not a grant.
Every entry must fit inside the policy ceiling supplied to validation, and the ceiling is bound into the lock.
DNS behavior, protocols, and ports are not accepted by this schema version.

### `[lifecycle]`

| Key | Required | Accepted |
|---|---|---|
| `idle_timeout_seconds` | Yes | 1 through 2,592,000 (thirty days), and not more than `maximum_lifetime_seconds` |
| `maximum_lifetime_seconds` | Yes | 1 through 2,592,000 |
| `on_idle` | Yes | `destroy`, `stop`, or `checkpoint`, and the Backend must support the chosen action |

The thirty-day bound is the Generation compiler's `MAX_TTL_SECONDS`, so every locked maximum lifetime is a valid compiler lifetime limit.

### `[[environment]]`

| Key | Required | Accepted |
|---|---|---|
| `name` | Yes | An ASCII letter or underscore, then letters, digits, or underscores, at most 256 bytes, unique across entries |
| `value` | One of `value` or `required = true` | Up to 4,096 bytes |
| `required` | One of `value` or `required = true` | `true` |

Up to 256 entries.
A `value` entry is a literal that the Template commits.
A `required = true` entry is a slot that Launch must fill.
A literal is rejected when a composed module declares the name as secret, when the value has a known credential shape, or when the name carries a credential marker such as `TOKEN`, `PASSWORD`, `API_KEY`, or `SECRET` as a whole underscore-delimited component and the value is not trivial.
`TOKENIZERS_PARALLELISM = "false"` is accepted; `GITHUB_TOKEN = "x"` is not.
The rejection never echoes the literal.

### `[[secrets]]`

| Key | Required | Accepted |
|---|---|---|
| `name` | Yes | An environment name, unique across secrets |
| `source` | Yes | `secret://` followed by a non-empty graphic ASCII path with no credential inside it |
| `delivery` | Yes | `environment`, `file`, or `egress-proxy` |
| `scope` | Depends on `delivery` | `environment`: optional, defaults to `name`, must be an environment name; `file`: required, an absolute normalized guest path; `egress-proxy`: required, a domain inside the egress envelope |
| `mode` | `file` only | An owner-only mode such as `0o400` or `0o600`; default `0o400` |

Up to 64 entries.
Write the mode as a TOML octal literal.
`mode = 420` is the decimal number 420, which is `0o644`, and it is rejected because group and other bits are set.
Delivery targets are exclusive: two secrets may not share one environment name, guest file, or destination, a secret may not target a name that `[[environment]]` or a module seal already fills, and a file secret may not land inside a path a module owns.
A Template stores references only; no secret value exists anywhere in the compiler.

### Compiling a document today

The library entry points are `soma_template::parse_template`, which turns document bytes into a `Template`, and `soma_template::resolve`, or `resolve_with` for a custom module registry, which turns a `Template` into a `TemplateLock`.
There is no `soma template` command, and the `OciResolver` and `FilesystemOracle` traits have no production implementation anywhere in the workspace, so no shipped code can resolve a real Template against a registry or inspect a real root filesystem.
The only way to compile a document today is a Rust test that supplies `TestResolver`, `PolicyCeiling`, `BackendCapabilities`, and `TestFilesystemOracle`; the pattern is in `crates/soma-template/tests/support/mod.rs`.
`cargo test --locked -p soma-template` is the executable reference: 94 integration tests, all passing at this revision.

## 4. Modules

A module is convenience configuration expressed as data.
It declares what it owns, what it needs, and where it can run.
No module carries VMM behavior, and no agent brand receives special treatment.

### The built-in registry

`ModuleRegistry::builtin()` holds exactly four modules at this revision.

| Module | Owns | Exports | Environment | Destinations | Health probe | Default command |
|---|---|---|---|---|---|---|
| `soma://agent/claude-code@1` | `/usr/local/bin/claude`, `/usr/local/lib/soma/agents/claude-code` | `/usr/local/bin/claude` | Requires `ANTHROPIC_API_KEY`, which must never be a literal | `api.anthropic.com:443` | `/usr/local/bin/claude --version`, 30 s | `claude` |
| `soma://agent/osa@1` | `/usr/local/bin/osa`, `/usr/local/lib/soma/agents/osa` | `/usr/local/bin/osa` | `OSA_API_KEY` must never be a literal; not required | none | `/usr/local/bin/osa --version`, 30 s | `osa` |
| `soma://tools/git@1` | `/usr/bin/git`, `/usr/lib/git-core` | `/usr/bin/git` | Seals `GIT_TERMINAL_PROMPT=0` | none | `/usr/bin/git --version`, 10 s | none |
| `soma://tools/shell@1` | `/usr/local/lib/soma/tools/shell` | `/bin/sh`, `/bin/bash` | none | none | `/bin/sh -c true`, 10 s | none |

Every built-in module supports `linux/amd64` and `linux/arm64`.

These modules are contracts only.
The data says what a module will own and export once the deterministic build plan and Generation construction slices exist, and the compiler trusts an exported executable without consulting the filesystem oracle.
Listing `soma://tools/shell@1` therefore makes `program = "bash"` pass the executable check today, but no build step puts a shell into the image yet.
Until tickets T6 and T7 of [the implementation map](../research/template-implementation-map.md) are implemented, the only files inside a sandbox are the ones in your image.

There is no way to define a module inside a Template document.
A custom registry can be supplied through the Rust API `resolve_with`, and a content-addressed module store is a later slice.

### Composition order

Composition is a flat ordered list, not an inheritance tree.
The rules are fixed:

- Modules are visited in the order you wrote them.
- A module's transitive `requires` are placed before it, in their declared order, and each module appears once.
- Listing a transitive requirement yourself does not change the composed order.
- A reference without a version, an unknown module, a module listed twice, or a cycle stops composition with the responsible module and field named.
- The composed order is bound into the content digest, so two Templates that list independent modules in different orders have different `LockId` values.

After ordering, composition checks conflicts: two modules may not claim one exclusive field, two modules may not own overlapping guest paths, two modules may not seal one environment name to different values, and when `[command]` is omitted at most one distinct default command may exist.

### Sealed environment values

A module may seal an environment value.
`soma://tools/git@1` seals `GIT_TERMINAL_PROMPT=0`.
The Template may restate the same value, which changes nothing, but a different value is rejected with the sealing module named.
A secret may not deliver into a sealed name.
The design states that Launch may not override a sealed value either; the Launch-narrowing proof is still open under ticket T4.
The lock records each sealed entry with the module that sealed it.

### Deny by default

Every Template defaults to denied egress and denied ingress, not only those with agent modules.
A module destination such as the `api.anthropic.com:443` declared by `soma://agent/claude-code@1` is information about what the agent wants to reach.
It never widens the envelope.
A Template that composes `soma://agent/claude-code@1` without a `[network]` table locks `Deny` for egress, and `allows_domain("api.anthropic.com")` on that lock is false.
To let the agent reach its API you must write `allow_domains = ["api.anthropic.com"]` yourself, and the policy ceiling must permit it.

## 5. Deciding what goes inside

### Sizing

Every number below is labeled with its boundary and build type and comes from [the evidence directory](../evidence).
None of them is a benchmark.

| Quantity | Measured value | Boundary and build |
|---|---|---|
| EROFS root of `busybox:stable-musl` | 1,511,424 bytes from 424 tree entries | Generation compiler output on the Linux x86_64 host, erofs-utils 1.9.4 |
| EROFS root of `node:22` | 1,129,172,992 bytes from 33,512 tree entries | Same compiler and host; the source OCI layout export was 802 MB |
| Guest RAM touched, busybox, 256 MiB registered | 32,168 kB | Cold boot through one `uname -a`, debug build, single sample, inside a container |
| Guest RAM touched, `node:22`, 1 GiB registered | 67,468 kB | Cold boot through one `node --version`, debug build, single sample, inside a container |
| Host process resident size, busybox | 46,800 kB peak | The test process driving the machine, not a production `soma-vmm`, debug build, single sample |
| Host process resident size, `node:22` | 100,648 kB last sample | Same test process; it started the run at 32,608 kB because compiler buffers were still resident |
| Non-guest host overhead of the test process in the PVH kernel-boot proof | Below 1 MiB anonymous, about 3.6 MiB resident with file-backed code | Debug build, single sample, one 1-vCPU guest with 256 MiB, excluding the kernel image buffer and guest-touched pages |

The 64 MiB per-VM placeholder in [the visual atlas](../architecture/visual-atlas.md) must not be replaced by these numbers without a release-build, multi-sample measurement of the real `soma-vmm` process.

The CLI and MCP shape default is 1 vCPU, 1,024 MiB of memory, and 10,240 MiB of writable storage.
The Template document has no defaults, and the Generation compiler profile version 1 accepts one vCPU, 128 MiB through 3 GiB of memory, and writable storage of at least 64 MiB in 4 MiB units.
The two live runs used 1 vCPU with 256 MiB and a 64 MiB writable class for busybox, and 1 vCPU with 1 GiB and a 1 GiB writable class for `node:22`.
In the production design, writable storage is a size class: the sterile ext4 template is prepared once per class, and [the XFS reflink storage profile](../research/xfs-reflink-profile.md) implemented in `crates/soma-storage` gives each Instance a private copy-on-write head cloned outside Launch, so the number you write is a limit rather than bytes consumed at Launch.
The live-proved KVM runs do not do this; they copy the whole sterile template into a private head before boot, so on that path the bytes really are spent.

Memory is the resource to size carefully.
The guest kernel touches only the pages it uses, but the Machine shape reserves the whole amount for admission, and for large language runtimes [the capacity model](../architecture/visual-atlas.md#16-can-one-host-create-100000-sandboxes) names resident RAM and private dirty pages as the likely first constraint.

### Choose a slim base image

The immutable root is stored once per Generation and shared read-only by every Instance, so image size is not per-Instance RAM.
Image size does cost build time, storage, page cache, and the time to load a binary through the root block device.
The complete `node:22` test took 369 s of wall time on the Linux host, almost all of it in OCI import, normalization, EROFS formatting, independent verification of the 1.1 GB tree, and the 1 GiB overlay template build, in one debug-build test run.
Loading Node from the 1.1 GB root and reporting its version took 39.6 ms in that run, against a 7.5 ms busybox command round trip, both debug-build cold-boot single samples.
A `-slim` image variant or a distroless image is a better base than a full distribution image when the workload does not need the extra packages.

### Put runtimes in the image, not at Launch

SOMA does not install a Workload runtime during Launch, and nothing in the schema asks it to.
If the agent needs Node, Python, Git, a compiler, or a shell, build an image that contains it, push it, and select it under `[workload]`.
The image's layers are the reproducible customization mechanism; the Template selects and constrains, it does not install.

### Keep secrets as references with a delivery mode

Write `source = "secret://..."` and choose the delivery.
`environment` is for programs that read a credential from a variable.
`file` is for programs that need a credential file, with an owner-only mode.
`egress-proxy` keeps the credential outside the guest and scopes it to one destination inside the envelope; it is the strongest containment when the upstream protocol can be mediated.
Any literal that looks like a credential, in any bound field, is rejected before it can reach a lock.

### Narrow the network to an allowlist

Start from the default, which is deny everything.
Add exactly the domains the workload must reach under `allow_domains`, and addresses only under `allow_cidrs` with an explicit prefix.
Keep `ingress = "deny"` unless the workload serves requests.
The envelope is a maximum that the organization ceiling must permit and that Launch may later narrow; nothing may widen it.

The production enforcement design is [the Linux network profile](../research/linux-network-profile-v1.md), whose live proof in a container showed public egress, declared DNS, and drops for cloud metadata, undeclared resolvers, the Host, and peer guests.
Domain-level allowlists are not yet compiled into that enforcement, and the portable request contract has no allowlist class, so a locked allowlist envelope cannot be handed to the Generation compiler today; see section 7.

## 6. The ten things the Template compiler will reject

The compiler reports exactly one rejection, the first in validation order, as `[<module> ]<field>: <reason>`.
The field is a dotted path with list indexes, and the module prefix appears when a module rather than the Template is responsible.
Each row below edits the specification example from [the template system document](../architecture/template-system.md) and was run against the test resolver, ceiling, Backend, and oracle in `crates/soma-template/tests/support/mod.rs`.

| Class | One edit that triggers it | Error text |
|---|---|---|
| Unresolvable image | `image = "python:3.13-slim"` when the resolver has no digest for it | `` workload.image: image `python:3.13-slim` cannot be resolved for platform linux/amd64 `` |
| Incompatible module or platform | Remove the `[[secrets]]` entry for `ANTHROPIC_API_KEY` | `` soma://agent/claude-code@1 required_environment[0]: environment `ANTHROPIC_API_KEY` is not provided `` |
| Exclusive conflict | Add `"soma://agent/osa@1"` to `modules` and delete `[command]` | `soma://agent/osa@1 default_command: conflicts with the default command of soma://agent/claude-code@1` |
| Secret literal | `value = "ghp_1234567890"` on the `CI` entry | `` environment[0].value: `CI` carries a secret literal `` |
| Secret without scope | `delivery = "file"` with no `scope` | `` secrets[0].scope: secret `ANTHROPIC_API_KEY` lacks a delivery scope `` |
| Network exceeds ceiling | `ingress = "unrestricted"` under a ceiling that denies ingress | `` network.ingress: `unrestricted` exceeds the policy ceiling `deny` `` |
| Executable absent | `program = "nope"` | `` command.program: executable `nope` is absent from the resolved filesystem `` |
| Invalid value | `vcpus = 0` | `resources.vcpus: must not be zero` |
| Unsupported lifecycle action | `on_idle = "checkpoint"` when the Backend supports only destroy and stop | `` lifecycle.on_idle: `checkpoint` is unsupported by the Backend `` |
| Module graph | `"soma://tools/git"` without `@1` | `` modules[1]: `soma://tools/git` is unpinned `` |

More shapes of the same classes:

- `platform = "linux/arm64"` against a Backend that admits only `linux/amd64` gives `workload.platform: platform linux/arm64 unsupported by the Backend`, and a module that lacks the platform names itself with the field `platforms`.
- A second `[[environment]]` entry `GIT_TERMINAL_PROMPT = "1"` next to `soma://tools/git@1` gives `` environment[1].value: `GIT_TERMINAL_PROMPT` is sealed by soma://tools/git@1 ``.
- `allow_domains = ["api.anthropic.com", "evil.example"]` gives `` network.allow_domains[1]: `evil.example` exceeds the policy ceiling `3 permitted domain patterns` ``.
- `idle_timeout_seconds = 20000` with a 14,400 s maximum gives `lifecycle.idle_timeout_seconds: idle timeout must not exceed the maximum lifetime`.
- `mode = 420` on a file secret gives `secrets[0].mode: must be an owner-only file mode`.
- `allow_cidrs = ["10.0.0.1/24"]` gives `network.allow_cidrs[0]: must be an IPv4 or IPv6 CIDR` because a host bit is set.
- A cycle or an unknown transitive requirement names the requiring module and its `requires[<index>]` field.

Parse errors come earlier and look different: `` unsupported template schema `soma.template/v2` ``, `` missing field `resources` ``, `` unknown field `workload.tag` ``, `` field `lifecycle.on_idle` is invalid: expected destroy, stop, or checkpoint ``, and `` field `environment[0].value` is invalid: declare exactly one of `value` or `required = true` ``.
A `modules` list written after the `[workload]` header is reported as `` unknown field `workload.modules` ``.

A resolver or oracle that cannot answer is not a rejection.
It is reported separately as `OCI resolver unavailable: ...` or `filesystem oracle unavailable: ...`, so an outage never turns into a claim about your Template.

## 7. From Template to running sandbox

```text
Template  --compile-->  Template Lock  --build-->  Generation  --Launch-->  Instance  --Execute-->  Receipt
          in-process                    minutes                cold boot              bounded command
          no I/O                        OCI import, normalize,  or restore;           argv, timeout,
                                        EROFS, initramfs,       neither is wired        output limit
                                        verify, manifest        into a Launch yet
```

| Transition | What happens | What it costs, with boundary and build type | Status at this revision |
|---|---|---|---|
| Template to Template Lock | Parse, compose modules, pin the OCI digest through the resolver, validate against the ceiling, Backend, and oracle, encode `SOMALOCK` version 1, hash to the `LockId` | In-process work with no disk or network I/O; no timing was retained | Library path: `parse_template` then `resolve` in `crates/soma-template`, exercised by 94 tests plus the crate-boundary test in `soma-generation`; the resolver and oracle are test seams |
| Template Lock to Generation | Import and verify the OCI layout, normalize the layers into one tree, format the EROFS root, build the sterile overlay templates, verify the kernel, write the initramfs, encode the `SOMAGEN` manifest, hash to the `GenerationId` | `node:22`: 369 s wall for the whole test on the Linux x86_64 host, one debug-build run; OCI import verification of an extracted `node:22` ARM64 layout: 28.1 s and 27.9 s on the development Mac | Component-tested for phases 1 through 3 and 6; certification is designed only, so no Generation is launchable in the production sense; the compiler consumes the lock through the `TemplateRevision` view only for one vCPU and only for a fully denied or unrestricted envelope |
| Generation to Instance | Create the VM, map RAM, attach the five virtio devices, write the launch page, boot the kernel, run the Guest agent as PID 1, compose the root, consume the page, handshake over vsock, Repair, retire the page, pass the fixed probe, report Ready | `Ready` 129 ms after `KVM_RUN` for `node:22`, 164 ms and 193 ms in two busybox samples; debug build, cold boot, single samples, busy host, inside a container | Live-proved at `71161ea`, historical: the ignored `x86_64_sandbox_boot` test in `crates/soma-kvm` on a Linux x86_64 host with `/dev/kvm`, the pinned kernel, erofs-utils 1.9.4, and the static Guest agent, run before initramfs layout v3 and launch-page schema 3; no network egress, no jail, no prepared worker. Snapshot capture and restore are live-proved separately at `7c1127d` and equally historical, and neither path starts from a Template Lock |
| Instance to Receipt | One authenticated bounded Execute, then an authenticated Shutdown and orderly reset | `node --version` round trip 39.6 ms and busybox `uname -a` 7.5 ms, same samples | The test drives `soma-guest` directly; the default command locked in the Template is not started by any code yet |

Build time is paid once per Generation.
Launch time is paid once per Instance.
Command time is paid once per Execute.
The 10 ms targets in [the benchmark contract](../benchmark-contract.md) apply to a prepared Launch boundary on a certified Host that restores a Snapshot; that boundary is designed, no admitted measurement of it exists, and it must never be read into the cold-boot numbers above.

### What you can run today

The CLI has `soma run <image> -- <argv>`, `soma machine launch|exec|inspect|stop|destroy`, `soma doctor`, and `soma version`.
The MCP server has `soma_doctor`, `soma_run`, `soma_launch`, `soma_exec`, `soma_inspect`, `soma_stop`, and `soma_destroy`.
`soma run` and `soma machine launch`, and the `soma_run` and `soma_launch` tools, take an OCI image reference and a Machine shape; `exec`, `inspect`, `stop`, and `destroy` take an Instance identity, and `doctor` takes only `--strict`.
None of them reads a Template document, and the Docker and Apple Backends behind them create a container or an Apple VM directly from the image rather than from a Generation.
On macOS a `node:22` one-shot through Docker Desktop took about 1.19 to 1.24 s end to end across five runs, and through the Apple Backend 1.995 s end to end with 17.2 ms from machine launched to command ready, both development-host measurements of a container and an Apple VM respectively, not of the SOMA VMM.

Until a `soma template` command or a Template-accepting Launch exists, a Template is a reviewable statement of intent that the library can compile to a `LockId`.
Everything the lock says about environment, secrets, working directory, user, and lifecycle is recorded but not yet delivered into a guest.
A bare-name `program`, including the `claude` and `osa` module default commands, is locked but cannot be executed, because the application wire contract version 1 requires an absolute path.

## 8. Worked examples

Each document below parses and resolves to a lock at this revision against a resolver, ceiling, Backend, and oracle that admit the referenced images, domains, and executables.
Whether the image really contains the program is the oracle's answer; today only the test oracle exists, so treat the executable check as a promise the real oracle will keep, not as proof about a registry image.

### A Node agent sandbox

The agent is compiled into its own image, so the Template only selects and constrains.

```toml
schema = "soma.template/v1alpha1"
name = "node-agent"
description = "A Node.js agent built into its own image"

[workload]
image = "registry.example.com/agents/node-agent:1.4"
platform = "linux/amd64"

[command]
program = "/usr/local/bin/node"
args = ["/opt/agent/main.js"]
working_directory = "/opt/agent"

[resources]
vcpus = 1
memory_mib = 1024
writable_storage_mib = 1024

[network]
egress = "allowlist"
allow_domains = ["api.example.com"]
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 600
maximum_lifetime_seconds = 7200
on_idle = "destroy"

[[environment]]
name = "NODE_ENV"
value = "production"

[[environment]]
name = "AGENT_TASK"
required = true
```

The lock records `NODE_ENV=production` as a literal and `AGENT_TASK` as a slot with no value that Launch must fill.
The command is locked as `/usr/local/bin/node /opt/agent/main.js` in `/opt/agent` as `root`.
The envelope is an allowlist of one domain, which the ceiling must permit; because the portable request contract has no allowlist class yet, `TemplateRevision::shape()` on this lock fails closed with `UnrepresentableNetwork` instead of guessing.

### A Python plus Git sandbox with an API key delivered by environment

```toml
schema = "soma.template/v1alpha1"
name = "python-git-tools"

modules = ["soma://tools/git@1"]

[workload]
image = "python:3.12-slim"
platform = "linux/amd64"

[command]
program = "/usr/local/bin/python3"
args = ["-m", "agent"]
working_directory = "/workspace"
user = "agent"

[resources]
vcpus = 1
memory_mib = 2048
writable_storage_mib = 4096

[network]
egress = "allowlist"
allow_domains = ["api.service.example", "github.com"]
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 900
maximum_lifetime_seconds = 14400
on_idle = "destroy"

[[environment]]
name = "PYTHONUNBUFFERED"
value = "1"

[[secrets]]
name = "SERVICE_API_KEY"
source = "secret://vault/service/api-key"
delivery = "environment"
```

The lock carries two environment entries sorted by name: `GIT_TERMINAL_PROMPT=0` sealed by `soma://tools/git@1`, and `PYTHONUNBUFFERED=1` from the Template.
The secret is locked as a reference to `secret://vault/service/api-key` with `environment` delivery and the scope `SERVICE_API_KEY`, which defaulted to the name.
Writing `value = "sk-..."` instead of a secret reference is rejected as a secret literal.
`github.com` is in the allowlist because Git needs it; the module does not add it for you.
The `user = "agent"` line locks the name only, and today's Guest agent still runs commands as root.

### A deny-all offline sandbox

This is the shape of the retained busybox run.

```toml
schema = "soma.template/v1alpha1"
name = "offline-busybox"

[workload]
image = "busybox:stable-musl"
platform = "linux/amd64"

[command]
program = "/bin/busybox"
args = ["uname", "-a"]

[resources]
vcpus = 1
memory_mib = 256
writable_storage_mib = 64

[network]
egress = "deny"
ingress = "deny"

[lifecycle]
idle_timeout_seconds = 60
maximum_lifetime_seconds = 600
on_idle = "destroy"
```

The `[network]` table is written out for clarity and could be omitted with the same result.
This lock projects exactly onto the isolated portable network policy, and its shape of 1 vCPU, 256 MiB, and 64 MiB is inside the Generation compiler profile version 1.
A Generation compiled from `busybox:stable-musl` with this shape booted on a real Linux x86_64 host, reached Ready 164 ms and 193 ms after `KVM_RUN` in two debug-build cold-boot samples, and returned the exact bytes of `uname -a` with exit status 0.
The network device in that run sat behind a link-down loopback backend, so no frame left the machine, which is the same result a denied envelope must produce once the network profile is attached.

## 9. Words you may bring from Docker, Firecracker, or E2B

| You may say | SOMA says | The difference that matters |
|---|---|---|
| Image | OCI image | An input to a Generation; SOMA never launches an image directly |
| Tag such as `node:22` | Mutable alias | The Template Lock pins the exact manifest digest for one platform |
| Dockerfile | Dockerfile | Still how you build the image; SOMA does not read it, and a Template contains no build steps |
| Container | Instance of a Machine | A hardware-virtualized guest with fresh identity per Launch; the Docker Backend is the development exception and says so in its Receipt |
| `ENTRYPOINT` and `CMD` | `[command]` or a module default command | The image configuration is not consulted |
| `docker run -e NAME=value` | `[[environment]]` and `[[secrets]]` | Credentials must be references; literals are rejected |
| `--cpus`, `--memory` | Machine shape under `[resources]` | vCPU count, MiB of memory, and MiB of writable storage, with no defaults in the document |
| `--network none` | `egress = "deny"` and `ingress = "deny"` | This is the default, and an allowlist is a maximum the ceiling must permit |
| Volume or bind mount | Workspace volume | Outside the alpha; the writable root is private and disposable |
| Template in E2B, or a golden image in Firecracker | Generation | A SOMA Template is the recipe; the Generation is the prepared immutable artifact |
| Snapshot in Firecracker | Snapshot inside a Generation | Not launchable by itself, and none has been captured yet |
| microVM or VM | Machine | One `soma-vmm` process per Machine in the production design |
| Sandbox in E2B | Sandbox, which maps to one Machine | The user-facing word for one Machine; each Launch gives it a fresh Instance |
| rootfs as one ext4 file | Immutable EROFS root plus a private ext4 overlay head | The root is shared read-only and byte-reproducible; the head is per-Instance |
| `kernel_image_path` | The pinned Generation kernel | Not selectable per Template; the kernel and its command line are part of the `GenerationId` |
| `docker exec` or `commands.run` | Execute | An argument vector with a timeout and an output limit, answered with a Receipt |
| Warm pool, pause, resume | Prepared worker, `on_idle = "stop"` or `"checkpoint"` | Prepared workers are Host capacity policy; `checkpoint` needs a Backend that supports it |
