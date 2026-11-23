# Platform parity, and what the second platform actually is

Every capability added to SOMA in the current push landed on one platform: Linux x86_64 with KVM.
Meanwhile the repository carries 4,740 lines of macOS adapter across 41 source files, an ARM64 module inside `soma-kvm`, and three retained evidence artifacts produced on Apple Silicon.
Nobody has assessed any of that against the code as it stands today.

This document is that assessment.
It reads the code rather than the prose, and where the code and the prose disagree it says so with file references.
It exists because [the benchmark contract](../benchmark-contract.md) carries an explicit anti-gaming rule, "Do not compare ARM64 and x86_64 results as one execution path", which turns platform parity from a cosmetic question into a rule about what may be printed as one number.

Statements attributed to Apple below are claims from Apple's published command contract and from what its runtime reports, not facts SOMA measured.

## The short answer

There is no second platform in the sense the word usually means.
There are three separate things that get grouped under "not x86_64 Linux", and they are at three different maturities.

The macOS adapter is a subprocess wrapper around a third party command-line runtime.
It covers a container lifecycle and nothing of the SOMA machine contract.
It is not built or tested by the merge gate.

The ARM64 module inside `soma-kvm` is a crate-private, test-only KVM boot proof with its own bespoke guest protocol.
It is compiled on every pull request and executed by nobody.

The portable crates, meaning `soma`, `soma-guest`, `soma-template`, and the policy halves of `soma-hostd`, genuinely are portable, and their portability is real rather than aspirational.
They are also the half of the system that does not touch a virtual machine.

## What `soma-macos` implements

The crate's own description is "Development-only macOS OCI sandbox adapter for SOMA" (`crates/soma-macos/Cargo.toml`), and its module documentation says it "deliberately does not certify the production Linux KVM path" (`crates/soma-macos/src/lib.rs`).

Of the 41 source files, 28 files and 3,290 lines are the adapter and 13 files are its tests.
Every entry point begins with `ensure_host`, which returns `BackendError::UnsupportedHost` unless `cfg!(all(target_os = "macos", target_arch = "aarch64"))` held at compile time (`crates/soma-macos/src/backend/mod.rs`).
There is no runtime probe of the host beyond that compile-time constant, which is correct for the purpose and worth knowing when reading a failure.

The lifecycle it covers is a container lifecycle.
`probe` reads `container system version --format json` and `container system status --format json`, requires the CLI to report a version inside `>=1.3.0,<1.4.0`, and requires the runtime service to report `running` (`crates/soma-macos/src/backend/probe.rs`).
`run` executes one bounded command from an image and then proves force deletion before returning, treating a cleanup failure as taking precedence over a command failure because absence could not otherwise be proven (`crates/soma-macos/src/backend/one_shot.rs`).
`create`, `start`, `execute`, `stop`, `delete`, and `inspect` cover the managed path (`crates/soma-macos/src/backend/lifecycle.rs`).
Ownership is enforced by requiring the inspection document to name exactly one record whose identifier matches the deterministic container name and whose `io.miosa.soma.instance` label matches the instance identity, and every control operation inspects before acting (`crates/soma-macos/src/backend/ownership.rs`).
Networking is expressed as `--network none` or `--network default`, plus `--dns` and `--publish` arguments, and the result parser verifies that the configured and active network sets agree before reporting an attachment (`crates/soma-macos/src/backend/network.rs` and `ownership.rs`).

That is a carefully built adapter.
The care is real and the invariants it enforces are the right ones for what it is.

### What it does not do, against the Linux KVM path

The crate creates no virtual machine.
Every operation is an argument vector handed to Apple's `container` executable, and the virtual machine is created by Apple's runtime using Apple's Virtualization framework, which is a claim from Apple's documentation that SOMA does not verify.
[The local sandbox reality note](../architecture/local-sandbox-reality.md) already states this correctly: "The VM is therefore configured using SOMA parameters, but the VM itself is created by the Apple runtime and Virtualization framework."

Measured against the Linux KVM path, the following are absent from `soma-macos` entirely, and absent in the sense that no seam exists rather than in the sense that a seam is unimplemented.
There is no Generation and no Candidate, so nothing certifies what boots.
There is no snapshot, no capture, and no restore, so the entire prepared-restore mechanism that produces SOMA's only interesting latency numbers has no macOS analogue.
There is no launch page, so there is no fresh per-Instance responder authority.
There is no `soma-guest` session, so the guest is not authenticated, does not prove readiness, and has no identity that the host can bind a network lease to.
There is no guest agent at all, which is discussed on its own below.
There are no virtio device models, no prepared worker, no capacity admission, no network broker lease, no jail, no reflink head, and no Instance ownership.
The sterile boundary of ADR 0033 cannot even be expressed here, because there is no machine whose authority could be withheld and later assigned.

The correct summary is that the macOS adapter implements the outer shape of the portable lifecycle contract and none of the machine contract underneath it.

### It is not on the merge gate

`.github/workflows/ci.yml` runs on pull request and on push to `main`.
It has one job, Ubuntu 24.04 x86_64, which runs `./scripts/check.sh linux` and then a cross compile check against `aarch64-unknown-linux-gnu`.
The macOS job lives in `.github/workflows/portability.yml`, whose triggers are a weekly cron at `41 7 * * 1` and manual dispatch.

So the 4,740 lines of macOS adapter are compiled and tested once a week on a schedule, and never as a condition of merging.
A change that breaks the macOS build merges cleanly and is discovered up to seven days later.
The reviews already record this happening: [the public KVM Backend audit](../reviews/2026-08-30-public-kvm-backend-audit.md) notes that "Windows and macOS failed warnings-as-errors checks" on a preceding commit.
This is the single most consequential parity fact in the repository and no document currently states it.

## What the ARM64 module covers

`crates/soma-kvm/src/lib.rs` gates the module as `#[cfg(all(test, target_os = "linux", target_arch = "aarch64"))]`.
Three things follow from that one line.
It is test-only, so it is never part of a shipping binary.
It is crate-private, unlike `pub mod x86_64` immediately below it, so nothing outside `soma-kvm` can call it.
It is Linux ARM64, not macOS, so it is unrelated to the Apple adapter except that the retained runs happened to be produced inside a nested guest on an Apple host.

What it does cover is a genuine KVM boot.
It loads a PE-format ARM64 kernel through `linux-loader`, maps 128 MiB of guest RAM, creates one vCPU, configures a GICv3, builds a device tree, and emulates a 16550 console UART until an expected sentinel appears (`crates/soma-kvm/src/arm64/machine.rs`, `fdt.rs`, `gic.rs`, `uart.rs`).
Beyond cold boot it adds a second control UART carrying a bespoke framed protocol, four bytes of magic `SMAC` and a version byte, with five frame kinds and a 32 byte challenge in every header (`crates/soma-kvm/src/arm64/protocol.rs`), and an executor that sends one challenge-bound request, runs an absolute program without a shell, and collects bounded output and one terminal frame (`crates/soma-kvm/src/arm64/executor.rs`).
Containment is taken seriously: the module reserves `SIGRTMIN + 7`, unblocks it only inside `KVM_RUN`, and aborts the process rather than freeing memory a live vCPU could still reach (`crates/soma-kvm/src/arm64/mod.rs`).

What it does not cover is everything that makes a SOMA machine a SOMA machine.
There is no virtio anywhere under `crates/soma-kvm/src/arm64/`, so none of the five device models on the fixed bus exist on this path.
There is no reference to `soma_guest`, so the authenticated session, the launch page, and the readiness receipt are absent, and the guest side is C fixture code (`crates/soma-kvm/tests/fixtures/arm64_agent.c`, `arm64_init.S`, `arm64_process.c`) rather than the real agent.
There is no snapshot capture or restore.
The boot is a PE image boot rather than a PVH entry, so it shares no loader code with the x86_64 path.
And `crates/soma-local/src/backend/mod.rs` gates `mod kvm` to `all(target_os = "linux", target_arch = "x86_64")`, so even a bare-metal Linux ARM64 host has no KVM Backend behind the public contract at all.

Its two live tests are `#[ignore]` and demand `SOMA_KVM_ARM64_KERNEL` and `SOMA_KVM_ARM64_INITRAMFS` as absolute paths to existing files (`crates/soma-kvm/src/arm64/tests.rs`).
No workflow in `.github/workflows/` runs on a Linux ARM64 host, so those tests have never executed in CI and cannot.
What CI does do is compile them, through the `cargo check --workspace --all-targets --target aarch64-unknown-linux-gnu` step in `ci.yml`, which is why the module has not rotted.

The module's bytes are unchanged since the revision its command proof names.

## Which recent capabilities are x86_64 Linux only, and why

The useful distinction is between a capability that is architecture specific because it encodes architecture state, one that is Linux specific because it calls Linux interfaces, and one that is merely unimplemented elsewhere.
Conflating those three makes the parity problem look larger than it is in some places and much smaller than it is in others.

| Capability | Where it is gated | Nature of the gate |
|---|---|---|
| Sterile restore and snapshot restore | `crates/soma-kvm/src/x86_64/snapshot/restore/`, and `snapshot/kvm_state.rs` gates its bindings on `all(target_os = "linux", target_arch = "x86_64")` | Architecture specific by nature. It restores a register file, a local APIC, a CPUID policy, nested state, and a clock, all of which are architecture state |
| Launch page placement | `crates/soma-kvm/src/x86_64/launch_page.rs` | Architecture specific in placement, portable in content. The schema and its parsers in `crates/soma-guest/src/launch_page/` carry no target gate at all |
| virtio device models | `crates/soma-kvm/src/virtio/`, declared as an ungated `mod virtio;` in `lib.rs` | Mostly portable, contrary to expectation. The five device models, the queues, the chains, and the MMIO transport compile on every target. Only `FileBackend` and `OsEntropy` are `cfg(unix)` and one item is x86_64 gated. What is not portable is where the bus sits, how interrupts are routed, and how the guest is told any of it, all of which live under `x86_64/` |
| Filesystem protocol | `crates/soma-guest/src/application/filesystem/` | Merely unimplemented elsewhere. There is no `cfg(target_os)` or `cfg(target_arch)` anywhere in `crates/soma-guest` |
| Command context | `crates/soma-guest/src/application/command/context.rs` | Merely unimplemented elsewhere, on the same evidence |
| Guest-side execution of both | `crates/soma-guest-agent/src/filesystem.rs` and its siblings, each gated on `all(target_os = "linux", target_arch = "x86_64")` in `main.rs` | Architecture specific as written, by the author's own statement, because the repair modules encode kernel interface request layouts for exactly one target |
| Instance ownership | `crates/soma-hostd/src/instance/`, exported ungated; `pub mod daemon;` is `cfg(target_os = "linux")` | Merely unimplemented elsewhere for the policy, Linux specific for the socket. Nothing here is x86_64 specific |
| Network attachment seam | `crates/soma-kvm/src/virtio/devices/net/attachment.rs` | Portable. It is a backend swap and a MAC assignment with no target gate |
| The privileged network broker | `crates/soma-netd`, whose `lib.rs` gates essentially every module on `cfg(target_os = "linux")` | Linux specific by nature. Netlink, nftables, TAP devices, and network namespaces are Linux interfaces, not x86_64 ones. `scripts/netd-live-tests.sh` nevertheless refuses to run except on x86_64, which is a stricter gate than the code requires |

The pattern is clear once laid out.
The protocol layer is portable and already written portably.
The guest side of that protocol is nailed to one target by kernel interface layouts.
The machine layer is architecture specific for real reasons.
The host daemons are Linux specific for real reasons and x86_64 only by habit in their scripts.

## The guest agent on each platform

On Linux x86_64 there is a real agent.
It is `/init` of the deterministic initramfs, stays PID 1 for the life of the machine, and owns repair, the launch page, the authenticated vsock session, readiness, the executor, and the bounded filesystem operations.

On Linux ARM64 there is no agent.
`crates/soma-guest-agent/src/main.rs` gates every module on `all(target_os = "linux", target_arch = "x86_64")` and, on any other target, prints "soma-guest-agent runs only as Linux x86_64 PID 1" and exits with status 2.
The comment above that gate is explicit about why, saying the target is "the one target whose kernel interface request layouts the repair modules encode" and that `network_repair::target` refuses again if the gate is widened without verified layouts.
That is the correct design and it means widening the gate is a real engineering task, not a `cfg` edit.
What stands in for an agent on the ARM64 proof path is C fixture code speaking the `SMAC` UART protocol, which shares no code, no framing, and no authentication with the real agent.

On macOS there is no agent and no place to put one.
The guest is whatever the image's entrypoint is, running under Apple's runtime, and SOMA never speaks to anything inside it.
Every property the Apple path reports comes from Apple's inspection document, which is why [the diagrams note](../architecture/diagrams.md) says the Apple backend "reports only the properties that Apple Container 1.3 can actually enforce or verify".

## Stale and mislabeled ledger rows

The [claim ledger](../claim-ledger.md) states that it "is the single place that states what SOMA can do today".
Four problems appear when its rows are checked against the current bytes and against the artifacts they cite.

### The Apple row's status holds, its name does not

The row reads "Apple Virtualization Backend one-shot | Live-proved at `4d10493`".
The status term survives inspection.
`crates/soma-macos` has three commits in its entire history and none of them is after `4d10493`, and the artifact's own run revision `f9a7e1b` is an ancestor of `HEAD` with the crate unchanged since.
The bytes are the bytes that ran.

The name is wrong in a way that matters.
Nothing in `soma-macos` links or calls Virtualization.framework.
The crate spawns `container` and parses its JSON.
Calling the row "Apple Virtualization Backend" credits SOMA with driving a hypervisor it never touches, and it is the only place in the repository that does so, since `local-sandbox-reality.md` and `module-map.md` both describe the adapter accurately.
The row should name the Apple `container` command contract, which is what the code is pinned to and what `SUPPORTED_CONTAINER_VERSION_REQUIREMENT` fails closed against.

### Four rows were proved on ARM64 and none of them says so

[The Apple one-shot artifact](../evidence/2026-08-29-apple-node22-one-shot.md) records the host as an Apple M3 Ultra running macOS on ARM64 and the resolved OCI platform as `linux/arm64/v8`.
[The Node 22 OCI import artifact](../evidence/2026-08-29-node22-oci-import.md) records the same host and the same `linux/arm64/v8` platform, and it backs two rows: "Bounded verified OCI layout import" and "Deterministic normalized logical rootfs".
[The Docker artifact](../evidence/2026-08-29-docker-node22-local.md) records "Host: Apple Silicon macOS" with a "Docker Desktop, Linux ARM64 engine".

None of those four rows names an architecture.
The two import rows sit in the build-time pipeline section, directly upstream of the Generation compiler that feeds the x86_64 KVM path, and the honest reading is that SOMA's OCI import and rootfs normalization have a live proof on ARM64 and no live proof on x86_64.
Their code is otherwise unchanged in behavior: the only edit to `crates/soma-generation/src/normalize*` since `4d10493` is `e19bdde`, which widened two constants from `pub(super)` to `pub(crate)` and altered nothing else, so the status terms stand.
The architecture does not stand, because it is not stated at all.

The Docker case is sharper still.
"Docker Backend local sandbox lifecycle" was proved on an ARM64 Docker engine, and "Burst harness enforcing the benchmark contract", which is qualified "against the Docker Backend only", was measured on an Intel Core Ultra 9 275HX under Ubuntu x86_64 according to [its artifact](../evidence/2026-08-30-burst-harness-dry-run.md).
Two rows say "the Docker Backend" and name two architectures, and nothing in the ledger tells a reader they are two execution paths.
That is precisely the shape the anti-gaming rule exists to forbid, arrived at by accident rather than by intent.

### Two retained results have no row at all

[The ARM64 nested KVM cold-boot proof](../evidence/2026-08-28-arm64-kvm-cold-boot.md) and [the ARM64 nested KVM challenge-bound command proof](../evidence/2026-08-28-arm64-kvm-command-proof.md) are retained, are careful about their own boundaries, and name revisions that are ancestors of `HEAD` whose `crates/soma-kvm/src/arm64` bytes are unchanged since.
Neither has a ledger row.

Meanwhile [the module map](../architecture/module-map.md) lists, among what the current alpha contains, "explicit-fixture ARM64 KVM cold-boot and challenge-bound direct-command proofs".
So the repository advertises a capability in an architecture document that the ledger, which claims to be the single place stating what SOMA can do, does not record.
Either the module map is claiming something the ledger will not stand behind, or the ledger is missing two rows.
On the evidence, it is the second: the runs happened, the artifacts are good, and the rows were never written.

### The `soma-macos` row understates its own fragility

No row anywhere records that the macOS adapter is outside the merge gate.
A "live-proved" row whose code is verified once a week on a cron carries a different kind of confidence from one whose code is verified on every pull request, and the ledger has no way to say so.
This is not a status-term problem, since the term is about the run and not the gate.
It is an evidence-column problem, and the evidence column is prose, so it can simply say it.

## The contradiction about macOS, stated rather than smoothed

Four documents give macOS four different statuses, and the words they use are load-bearing.

`crates/soma-macos/Cargo.toml` says "Development-only".
[The portability contract](../architecture/portability.md) puts "Apple Silicon development adapter" in the local engine row and says separately that "The first supported substrate is Ubuntu 24.04 x86_64 on a KVM-capable host" and that "Additional environments become supported only after passing the same conformance suite".
[The beginners guide](../architecture/beginners-guide.md) gives the Apple backend the host requirement "Supported macOS host" and the intended role "Local hardware-VM development".
[The local sandbox reality note](../architecture/local-sandbox-reality.md) calls the Apple backend "The primary non-Docker local path".

"Development-only", "supported", and "primary" are three different claims.
The portability contract's own five-level capability ladder resolves which one is true.
Level one is client conformance, and macOS passes it.
Level two is KVM conformance on the exact Linux architecture, which macOS cannot reach in principle because there is no `/dev/kvm` on Darwin.
Levels three through five are unreachable for the same reason.
By the repository's own definition of support, macOS is at level one of five and cannot climb, so it is not a supported local engine and never will be under this ladder.

The word "supported" in the beginners guide is describing the host requirement of Apple's runtime, not SOMA's support level, and it reads as the latter.
The word "primary" in the local sandbox reality note is true within its own table, which is a table of what works on a Mac, and it reads as a claim about SOMA's priorities.
Both are locally defensible and jointly misleading, which is the usual way this kind of drift happens.

## The position

**macOS is a development convenience, not a supported target and not a research profile.**

It is not a supported target because it fails the repository's own conformance ladder at level two and implements none of the machine contract.
It is not a research profile either, because a research profile implies an open question somebody intends to answer with it, and nothing about the macOS path is being measured to decide anything.
It is a way for a person with a Mac to exercise the portable lifecycle contract against a real Linux virtual machine before pushing to a Linux host, which is a genuinely useful thing and should be described as exactly that.

The one qualification is that it is currently a development convenience with a weaker gate than the thing it is convenient for, which makes it less convenient than it appears.

**Linux ARM64 KVM is a research profile.**

That is the honest label for a crate-private, test-only module with two retained artifacts, its own guest protocol, no product path, and no CI host that can run it.
It answers a question, namely whether SOMA's boot and containment approach generalizes off x86_64, and the two artifacts answer it affirmatively for cold boot and one bounded command.
It is not a second engine and nothing about it is on a path to becoming one today.

**Windows is a client target only**, which `portability.md` already says and which the code agrees with.

## What would make two architectures reportable together

Under the anti-gaming rule, an ARM64 number and an x86_64 number may appear in one figure only when they are one execution path, and "one execution path" has to mean something checkable.
Four conditions make it checkable, and all four fail today.

The same measured binary and the same code path must exist on both, meaning the same public Backend driving the same VMM through the same seam.
Today ARM64 has no Backend at all, since `soma-local` gates `mod kvm` to x86_64.

The same Generation lineage must exist on both, since the Generation is what determines what boots and what is restored.
Today the Generation compiler produces x86_64 artifacts and there is no ARM64 Generation.

The same isolation class and the same preparation class must be observed and recorded per sample, not asserted.
The burst harness already binds `os.uname().machine` into every run's metadata record (`benchmarks/local_alpha/burst/metadata.py`) and `load_results` requires exactly one metadata record per results file (`benchmarks/local_alpha/burst/results.py`), so a single run structurally cannot mix architectures.
What is missing is any rule preventing a person from placing two such runs beside each other in one table, which is where the rule will actually be broken.

The same measured segment must be timed, from the same start event to the same end event.
The ARM64 command proof times a cold boot to a fixture sentinel over a UART; the x86_64 result times a prepared restore to a command return through the public command line.
Those are not the same segment and no arithmetic makes them comparable.

The practical consequence is a rule worth adopting now, before any ARM64 work resumes: **every latency figure and every ledger row names its architecture, and no report pools two architectures under one label.**
That rule costs one word per row today and prevents a class of error that would be very hard to detect later.

## Minimum work to give the second platform a defensible status

The point of this list is not parity.
Parity is not obviously worth buying.
The point is that each platform should have a stated status that the code supports, so that no reader has to do the work above to find out where they stand.

For macOS, treated as a development convenience, the minimum is four changes and none of them is large.
Rename the Apple ledger row so it names the Apple `container` command contract instead of the Virtualization framework, since that is what `soma-macos` is pinned to and fails closed against.
Add the architecture to that row and to the two OCI import rows, all of which were proved on `linux/arm64/v8`.
Either move the macOS job onto the pull-request gate or state in `portability.md` and in the ledger's evidence column that it is verified weekly rather than per change, because leaving that unsaid is the actual risk.
Reconcile "development-only", "supported", and "primary" to one word across `Cargo.toml`, `portability.md`, `beginners-guide.md`, and `local-sandbox-reality.md`.

For the Docker rows, add the architecture to both, since one was proved on ARM64 and the other on x86_64 while both are labeled "the Docker Backend".

For Linux ARM64, treated as a research profile, the minimum is two ledger rows recording the cold-boot and command proofs at the revisions their artifacts name, each stating plainly that the module is `cfg(test)`, crate-private, has no virtio, no snapshot, no `soma-guest`, and no public Backend.
That converts an undocumented module into a recorded research result, which is what it is, and it closes the gap where `module-map.md` advertises something the ledger does not carry.

Promoting Linux ARM64 from a research profile to anything higher is a much larger piece of work and should not be started without a reason.
It needs an ARM64 Generation, a widened guest agent with verified kernel interface request layouts for the target, virtio on the ARM64 machine, snapshot state for ARM64 registers and the GIC, an ARM64 Backend in `soma-local`, and a CI host that can run any of it.
That is a program of work, not a task, and nothing in the current roadmap depends on it.
