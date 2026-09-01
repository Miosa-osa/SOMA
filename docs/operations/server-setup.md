# Setting up SOMA on a server

This is the short path from a fresh Ubuntu host to a running development sandbox.
The operator or agent has SSH access and drives the result through the `soma` command line.
The current result is one cold-booted sandbox at a time, not a production service.

## Where this fits

SOMA is three layers, and this document exposes only the development form of the first layer:

1. **The machine engine**, linked into the command line and hosted API paths.
   The hosted API uses one dedicated machine-host process per sandbox.
2. **The prepared host pool**, which can restore one stopped identity-free VM and create one private disk per API slot before accepting traffic.
   Matching launches assign fresh authority and resume those machines, while nonmatching requests fall back honestly to on-demand restore.
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

Prepared artifacts and private heads for the fast profile must be on the same reflink-enabled XFS device.
Prove that invariant before starting the API:

```sh
scripts/check-fast-storage.sh /srv/soma/prepared /srv/soma/heads
```

The check creates and removes only a private 1 MiB sparse probe and its reflink clone.
It fails when the directories are on different devices, the filesystem is not XFS, or `FICLONE` is unavailable.
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

For latency qualification, every host in one cohort must use the same CPU frequency policy.
The east-host campaign found that mixing AMD P-state `powersave` and `performance` governors materially increased burst tail variance even when every host already selected the `performance` energy preference.
Record both `scaling_governor` and `energy_performance_preference` for every host, and normalize them before accepting benchmark evidence.

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

# the empty sandbox: prove it boots and runs a command
./target/release/soma --backend kvm run busybox:stable-musl -- /bin/busybox uname -a

# the node sandbox: a real runtime, running inside the VM
./target/release/soma --backend kvm run node:22 -- /usr/local/bin/node --version
```

To serve the prepared hosted path, select the exact Generation and memory class before starting the API:

```sh
export SOMA_PREWARM_REFERENCE=node:22
export SOMA_PREWARM_MEMORY_MIB=1024
./target/release/soma-api --listen 127.0.0.1:18787 --workers 64
```

The listener opens only after all 64 child processes have restored a stopped identity-free VM and created a unique unlinked private disk.
The worker count is both the HTTP concurrency bound and the prepared-slot target.
Size it from admitted host capacity rather than copying the qualification value.

For a durable host installation, install `deploy/systemd/soma-api.service`, copy `deploy/systemd/api.env.example` to `/etc/soma/api.env`, replace its paths with the certified host paths, install the exact release binary at `/opt/soma/bin/soma-api`, and link `/opt/soma/runtime` to the certified runtime directory that carries the kernel and runtime links.
The unit keeps the listener on loopback, restarts a failed process, and preserves the same explicit prepared-worker configuration used by the foreground command.

The Node command prints the version contained in the current `node:22` image.
Because `node:22` is mutable, do not expect one exact patch version unless the image is pinned by digest.
Successful output proves this development path reached guest command execution.
It does not independently certify the host, Candidate, cleanup, jail, networking, or production readiness.

The preparation command first publishes a non-launchable Candidate.
On Linux x86_64, `capture_snapshot` installs the captured objects, runs certification, promotes the exact Candidate into a ready Generation, and publishes `generation.id` last.
The public KVM resolver refuses Candidate-only entries and independently re-verifies the ready Generation and every bound artifact before machine creation.
There is no environment-variable bypass for Candidate launch.

## What this gives you, and what it does not

**What works today.** Hardware-isolated sandboxes restored from a certified snapshot and driven through the command line, MCP, or HTTP surfaces on a compatible KVM host, including a bounded prepared-machine HTTP path.

**What a production sandbox service still needs, and is not built yet:**

- Many concurrent sandboxes with capacity admission, so one host can serve a fleet safely.
- A jail around the virtual machine process.
  Today the VM runs inside the command line process rather than as a separately confined `soma-vmm` process.
  This is the most important gap before anything faces untrusted users.

The full path from this development setup to a production admitted service is tracked in [the public KVM Backend audit](../reviews/2026-08-30-public-kvm-backend-audit.md), the [MIOSA custom sandbox rollout plan](miosa-custom-sandbox-rollout.md), and the ticket map in [the VMM decision map](../research/vmm-decision-map.md).

## Teardown

Stop any running sandbox with the command line, remove the prepared store and head directories under `/srv/soma`, and if a tunnel was opened to reach the host, close it.
Rotate any credential that was shared to gain access.
