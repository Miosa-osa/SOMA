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

Eight frame kinds exist: `Prepare`, `Execute`, `Shutdown`, `RepairComplete`, `Stdout`, `Stderr`, `Terminal`, `ShutdownAck`, where `Terminal` is a command's exit status rather than a pseudo-terminal.

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

**The restored guest loses about 7 ms before its own code runs.** `RunStart` to `LaunchPageConsumed` is 7.0 ms on eval-1, and the guest's own clock reports none of it: with the launch-page wait turned into a spin, the agent still measures zero elapsed inside its loop. It is guest-kernel resume work and demand paging, invisible from both ends, and it is now the largest item in the ready segment. See [the eval-1 readiness split](../evidence/2026-08-31-eval1-ready-segment-split.md).

**Identity and network repair are 7 ms of the ready segment.** Writing the hostname, the machine identity, and the two session tmpfs mounts costs 3.9 ms, and installing the network identity costs 3.1 ms, both inside the guest. Neither is removable: they are exactly the per-Instance identity the pre-launch capture point exists to keep out of the image.

**The warm list is a heuristic.** The guest agent warms a conventional set of runtime paths because nothing tells it what the workload actually is. A compiler-emitted list from the Generation's entrypoint would warm exactly what runs and nothing else.

**The network hot path costs more than a whole sandbox launch.** The retained privileged run on eval-1 measured, at concurrency 100, network assign at 86.1 ms median and 370.7 ms at the ninety-ninth percentile, activation at 19.0 ms median, and release at 144.4 ms median with a 1.74 s maximum. The whole sandbox time to first command at that concurrency is 181 ms, so wiring the network in as it stands would roughly double it. Those figures were taken while the image matrix was running on the same host, so they are contended and need a clean repeat before anyone quotes them; the order of magnitude is the point.

The cost is mechanical rather than mysterious. `assign` renders a ruleset and applies it by executing `/usr/sbin/nft -f -` as a subprocess inside a scoped thread that has entered the bundle's namespace. At a hundred launches that is a hundred process spawns and a hundred ruleset parses on the request path. `prepare` already runs `nft` once, to install a fully denied ruleset, so the second application is the one that could move.

**That paragraph was measured afterwards and it is wrong about where the cost is.** One 15.9 ms ruleset application decomposes into about 0.3 ms of process spawn, about 1.2 ms of `nft` startup and parse, and about 14 ms of kernel transaction, which is an RCU grace period in the `nf_tables` commit path and is the same whether the transaction adds one rule or three chains. Entering the namespace, which the paragraph above implies is expensive, costs 0.06 ms.

So applying the ruleset over netlink removes about 1.5 ms of 15.9 ms, at the cost of encoding every rule expression by hand. It is not worth it, and the reason is recorded in the `nft` module so it is not rediscovered.

The cost that was worth removing was elsewhere and had not been noticed at all: the broker asked four read-only questions per lifecycle by running `nft list table`, each about 5.4 ms of which almost none is kernel work. Asking `NETLINK_NETFILTER` directly answers the same question in 0.02 ms, changes no policy property, and made activation 93 percent faster and release 19 percent faster. What remains in assign is a ledger fsync and one `nf_tables` commit, and neither can be improved from inside that crate.

The lesson is the one this document keeps relearning: the mechanism has to be measured before it is optimised, because the obvious explanation was wrong twice here, first about the spawn and then about the namespace.

**Cohort variance is 40 percent of the median.** Until that is understood, an optimisation smaller than about 60 ms cannot be distinguished from noise by a single hundred-way cohort, so measurement method is itself on the critical path for optimisation work.

## Not gaps, recorded so they are not rediscovered

- Sequential time to first command is 65.5 ms, inside the sub-90 ms band the category treats as table stakes.
- Restore, private overlays, per-Instance identity, authenticated repair, and proven cleanup all work and are retained.
- `soma-netd`, `soma-hostd`, `soma-jail`, and `soma-template` are substantially implemented. Their gap is integration, not absence.
