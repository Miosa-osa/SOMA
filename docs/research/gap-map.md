# The gap map

One place that names every gap between what SOMA is and what a sandbox product has to be, ordered so that each item is workable once the ones above it exist.
It is deliberately a map and not a plan: it says what is missing and why, and it does not commit to building any of it.

Companion documents: [the provider contract](provider-contract-gap-analysis.md) states the interface, [the capability survey](sandbox-provider-capability-survey.md) states the dimensions, and [the claim ledger](../claim-ledger.md) states what already works.

## How to read the order

Four gaps are structural: everything else waits on them, so their order is not a preference.

1. **Egress**, because a sandbox with no network can only run what was baked into its Generation.
2. **The guest protocol**, because the filesystem and terminal halves of the contract cannot exist above frames that do not carry them.
3. **A persistent Host Runtime**, because reconnect, list, and pause are ownership rather than API, and because the prepared worker path needs the same runtime.
4. **Shape**, because it is a decision rather than work, and it changes pool keying, admission, and the Template schema.

## 1. Egress

`soma-netd` implements namespaces, netlink, nftables, address management, DNS, ingress, activation, and reconciliation. `soma-kvm` implements a TAP backend. The KVM Backend uses neither and calls `link_down_network`, so no packet leaves any sandbox.

This is integration of two implemented subsystems, not new work, and it unblocks package installation, repository cloning, image pulls, and model API calls, which are the same missing thing wearing four names.

Open beyond the wiring: the placeholder MAC must become an admitted per-Instance MAC with a lease generation; egress policy has to be expressible in the Template; and a domain policy must not imply protocol coverage it does not enforce.

## 2. The guest protocol

Eight frame kinds exist: `PrepareAndProbe`, `Execute`, `Shutdown`, `RepairComplete`, `Stdout`, `Stderr`, `Terminal`, `ShutdownAck`, where `Terminal` is a command's exit status rather than a pseudo-terminal.

`GuestCommand` carries a program, arguments, a timeout, and an output bound. It carries no standard input, no environment, no working directory, no user, and no signal. Output is bounded and delivered when the command ends, so nothing streams.

Missing, and each needs a frame before it can need an API:

- the six filesystem operations the contract names, plus upload and download of files large enough not to fit one message
- an interactive terminal with resizable dimensions
- standard input, environment, working directory, and user per command
- streaming output rather than a bounded buffer at the end
- signalling or killing a running command
- more than one command in flight in one sandbox

The bound on message size is a real constraint here rather than an incidental one: a file API that only moves what fits in one authenticated message is not a file API, so framing has to come before operations.

## 3. Ownership

No Instance outlives the process that launched it, so `getById`, `list`, reconnect, pause, resume, and idle timeout are all absent for one reason.
[ADR 0031](../adr/0031-persistent-host-runtime-ownership.md) is the accepted answer and is not implemented.
The same runtime is what lets a prepared worker exist before demand, so ownership and the largest measured optimisation are one piece of work.

## 4. Shape

Each Generation has exactly one launchable shape: one vCPU by machine contract, memory that must equal the captured snapshot exactly, and an overlay from the Generation.
Offering a range means a Generation and a snapshot per shape, which is a decision about pool keying and admission before it is an implementation.

## 5. State that outlives an Instance

Every provider surveyed has a per-sandbox snapshot taken after a user has changed something. SOMA's snapshot is a build-time artifact shared by every Instance, and deliberately holds no tenant state.
A per-sandbox snapshot is a new object with a different lifetime and a different privacy class, and it has no design.
Attached and remote storage, which several providers offer, is the same question asked about disks.

## 6. Credentials

The Template schema carries `secrets` with a file mode. Nothing delivers a secret to a running Instance, and there is no host-side mediator, so neither documented delivery mode exists at runtime.
Keeping a token out of the agent's reach is table stakes in this category, and it is also the harder of the two modes.

## 7. Authoring

`soma-template` parses a TOML document and composes a Template Lock, and no example Template exists in the repository, and nothing builds a Generation from a Lock.
The authoring surface is the product's first impression, and it currently cannot be exercised at all.

## Optimisation, separately from features

These are measured or structural, and none of them is a feature gap.

**Machine construction, 48.0 ms at concurrency 100** against 2.71 ms uncontended. Entirely pre-claimable, and removed by a prepared worker pool. This is the largest single measured cost and it is already specified.

**The readiness probe is a full command round trip.** `PrepareAndProbe` runs a fixed probe inside the guest before an Instance is Ready. Whether readiness needs a whole execution, or can be proven by the repair receipt alone, is worth asking before optimising anything smaller.

**Handshake cost sits on the request path.** The host's ephemeral keypair is generated per session; generating it before a claim would move an X25519 keygen off the measured path without weakening the handshake.

**The warm list is a heuristic.** The guest agent warms a conventional set of runtime paths because nothing tells it what the workload actually is. A compiler-emitted list from the Generation's entrypoint would warm exactly what runs and nothing else.

**Cohort variance is 40 percent of the median.** Until that is understood, an optimisation smaller than about 60 ms cannot be distinguished from noise by a single hundred-way cohort, so measurement method is itself on the critical path for optimisation work.

## Not gaps, recorded so they are not rediscovered

- Sequential time to first command is 65.5 ms, inside the sub-90 ms band the category treats as table stakes.
- Restore, private overlays, per-Instance identity, authenticated repair, and proven cleanup all work and are retained.
- `soma-netd`, `soma-hostd`, `soma-jail`, and `soma-template` are substantially implemented. Their gap is integration, not absence.
