# Setting up SOMA on a server

This is the short path from a fresh Ubuntu host to a running development sandbox.
The operator or agent has SSH access and drives the result through the `soma` command line.
The current result is one cold-booted sandbox at a time, not a production service.

## Where this fits

SOMA is three layers, and this document exposes only the development form of the first layer:

1. **The machine engine**, currently linked into the CLI for this development path.
   The production design moves that engine into one jailed `soma-vmm` process per sandbox.
2. **`soma-hostd`**, per-host pools of prepared workers that let one host serve many sandboxes quickly instead of cold booting each.
   It is built and component-tested, but not yet wired to the live command-line path.
3. **The fleet control plane**, which places and admits sandboxes across many hosts. Designed.

This runbook stands up the development floor.
A production host also needs a certified host profile covering its host class, operating system, kernel, CPU, storage, network, SOMA build, isolation, cleanup, and performance evidence.
The support levels and full prerequisite contract are in [deployment portability](deployment-portability.md).

## Step 0: the server must be able to host a virtual machine

SOMA runs a real virtual machine per sandbox, so the host must expose Linux KVM.
The setup script checks the development prerequisites it can verify locally:

- **Linux on x86_64.** Ubuntu 24.04 is the tested target.
- **KVM** and the required CPU virtualization features.
  On bare metal this is normal; on a cloud VM it requires nested virtualization enabled by the provider.
- **cgroup v2** and kernel seccomp support.
- **A writable SOMA work root** for filesystem tools, prepared entries, and private heads.
- **sudo** to install packages and provision storage, and **network egress** to GitHub and a container registry.

After setup and a new login shell, run the KVM probe through the binary built in Step 3:

```sh
./target/release/soma --backend kvm doctor --strict
```

A passing strict doctor result proves the current KVM API probe only.
It does not prove cgroup containment, a VMM jail, private networking, storage cleanup, capacity admission, or production readiness.

**Recommended: an XFS filesystem with reflink** (`xfs_info <mountpoint> | grep reflink=1`).
Writable sandbox disks are then near free to create, which is what makes running many sandboxes cheap.
Without it each sandbox copies its disk instead, which still works but costs time and space.

Run every step below in order.

## Step 1: obtain the repository

Clone the repository with your own GitHub access.
If the target host has no GitHub credential, transfer a bundle made on a trusted machine:

```sh
# on a trusted machine with access
git fetch origin
git switch main
git merge --ff-only origin/main
git bundle create soma.bundle main           # bundle a real local branch
git bundle verify soma.bundle                # should report a complete history

# on the Ubuntu host
git clone -b main soma.bundle SOMA
cd SOMA
```

Bundle a real branch.
A bundle made directly from a remote-tracking ref such as `origin/main` carries the objects but no branch a clone can check out, so the clone succeeds and leaves an empty working tree.

No setup script contains or requests a repository credential.

## Step 2: prepare the host

```sh
./scripts/setup-host.sh
```

This installs host dependencies, enables Docker, adds the operator to the `kvm` and `docker` groups, installs the pinned Rust toolchain, and installs the guest-agent target.
It also provisions `/srv/soma/fs-tools`, `/srv/soma/prepared`, and `/srv/soma/heads` for the operator.
Required readiness failures make the script exit nonzero.
Open a new login shell afterward so group changes take effect, then return to the repository.

## Step 3: build SOMA

```sh
./scripts/build-soma.sh
```

This builds the three artifacts, the `soma` command line, the static guest agent, and the pinned guest kernel (about 60 seconds of compile), and prints the path and digest of each.

## The short path: one command

Steps 4 to 6 below are the manual sequence, and every step in it is a place to fail. Three of them
fail silently: a Candidate compiled but never captured cold boots instead of restoring, at about
fifteen times the time and with no error; a launch at a shape the snapshot was not captured at is
refused before a machine exists; and a store prepared against an older wire contract cannot launch
at all. None of the three says so.

`scripts/reproduce.sh` does the whole sequence, checks each precondition before it can fail
silently, and ends with a measured result:

```sh
./scripts/reproduce.sh --memory-mib 1024 --storage-mib 10240 --samples 25 \
    --expect v22 node:22 -- /usr/local/bin/node --version
```

It builds the workspace, the guest agent and both tools, compiles the image into a Generation,
captures its snapshot, verifies that the prepared store answers the shape and the wire contract of
the checkout in front of it, and then measures launches and prints percentiles and per-stage
medians. It refuses, naming the cause and the fix, when the kernel link dangles, `cargo` is off the
non-interactive PATH, the store is stale, the shape does not match, or the snapshot is missing.

Read the rest of this document to understand what it is doing, or when a step needs to be run on
its own.

## Step 4: build the pinned filesystem tools

```sh
./scripts/build-fs-tools.sh /srv/soma/fs-tools
```

The compiler accepts erofs-utils 1.9.4 and e2fsprogs 1.47.0 for this profile.
This script builds both from digest-verified source inside the pinned builder image instead of symlinking mutable host tools.

## Step 5: build your first Candidates

A **Candidate** is an OCI-derived machine image compiled with the pinned kernel and guest agent.
It becomes a production **Generation** only after snapshot installation, certification, and promotion.
Those library gates are implemented, but this setup flow still prepares only a development Candidate and does not yet run the live capture-and-promotion workflow.
The current development path can explicitly opt into booting the Candidate from the prepared store.

Prepare a small shell image and a Node runtime image:

```sh
# the empty sandbox: busybox, a shell and nothing else, a blank slate
./scripts/prepare-generation.sh busybox:stable-musl /srv/soma/prepared /srv/soma/fs-tools

# the standard template: node, a runtime ready to use
./scripts/prepare-generation.sh node:22 /srv/soma/prepared /srv/soma/fs-tools
```

Each call exports the image to an OCI layout, compiles the Candidate in a private staging directory, and atomically publishes one new prepared-store entry.
The tool refuses to overwrite an existing entry.
References are keyed by their SHA-256 digest so distinct registry references cannot collide through filename normalization.

## Step 6: run your first sandboxes

Point the writable disks at the reflink filesystem, then boot each template and run a command inside a fresh VM.
Start with the empty one, then the runtime:

```sh
export SOMA_GENERATION_STORE=/srv/soma/prepared
export SOMA_HEAD_DIR=/srv/soma/heads              # on the XFS reflink volume
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1        # see the note below

# the empty sandbox: prove it boots and runs a command
./target/release/soma --backend kvm run busybox:stable-musl -- /bin/busybox uname -a

# the node sandbox: a real runtime, running inside the VM
./target/release/soma --backend kvm run node:22 -- /usr/local/bin/node --version
```

The Node command prints the version contained in the current `node:22` image.
Because `node:22` is mutable, do not expect one exact patch version unless the image is pinned by digest.
Successful output proves this development path reached guest command execution.
It does not independently certify the host, Candidate, cleanup, jail, networking, or production readiness.

The `SOMA_ALLOW_UNCERTIFIED_GENERATION` flag is required on purpose.
Generation certification does not exist yet, so every template a host can build today is an unverified Candidate.
The backend refuses to launch one unless this flag is set, so that unverified images cannot boot by accident.
Setting it is the explicit opt in for a development host.

## Step 7: prepare the head root for concurrent launches

One sandbox at a time needs nothing further. A cohort of launches does: creating and unlinking a
head takes the head directory's inode lock, and cloning takes the refcount records of the
template's extents, so a hundred simultaneous launches against one directory and one template
queue on both.

```sh
# shard directories are free; the fan writes one copy of the template per copy asked for
./target/release/examples/fan_warm \
    --head-root /srv/soma/heads \
    --template <prepared entry>/snapshot/overlay.raw \
    --copies 4 --shards 16
```

The fan lives under the head root by default, so it is on the same filesystem as the heads, which
is what `FICLONE` requires. It costs one template's bytes per copy, 2 GiB each for a 2048 MiB
overlay, and it must be warmed once per template rather than once per launch: the tool reads
every copy back and refuses to publish one that is not the template byte for byte or that does
not own its own extents.

A launcher reads what this writes through `SOMA_HEAD_DIR`, `SOMA_HEAD_SHARDS` (16 by default) and
`SOMA_TEMPLATE_COPIES` (4 by default), with `SOMA_TEMPLATE_FAN_DIR` overriding where the fan is
looked for. A head root that was never warmed still launches: the clone falls back to the
template itself.

Sharding changes what the head root holds. A head is still created and unlinked inside one
launch, so no head file survives it, but the root now holds one directory per shard, and an
audit of heads against the ownership ledger reconciles each shard rather than the root.

## What this gives you, and what it does not

**What works today.** One hardware-isolated development sandbox at a time, cold booted and driven from the command line on a compatible KVM host.

**What a production sandbox service still needs, and is not built yet:**

- A daemon or API to call, rather than a one shot command.
  The backend currently tracks a single live sandbox.
- Many concurrent sandboxes with capacity admission, so one host can serve a fleet safely.
- A jail around the virtual machine process.
  Today the VM runs inside the command line process rather than as a separately confined `soma-vmm` process.
  This is the most important gap before anything faces untrusted users.
- Prepared restore, which reaches a ready sandbox in milliseconds instead of the current cold boot.
- Certified Generations, so a template is verified before it can launch rather than opted into.

The full path from this development setup to a production admitted service is tracked in [the public KVM Backend audit](../reviews/2026-08-30-public-kvm-backend-audit.md), the [MIOSA custom sandbox rollout plan](miosa-custom-sandbox-rollout.md), and the ticket map in [the VMM decision map](../research/vmm-decision-map.md).

## Teardown

Stop any running sandbox with the command line, remove the prepared store and head directories under `/srv/soma`, and if a tunnel was opened to reach the host, close it.
Rotate any credential that was shared to gain access.
