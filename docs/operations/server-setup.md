# Setting up SOMA on a server

This is the short version: what a server must already have, then the steps in order, from a fresh
machine to a running sandbox. The picture is an operator or an agent with SSH access to a plain
Ubuntu host, driving everything through the `soma` command line: no console, no manual VM
wrangling, just the CLI. It is a single-host development and evaluation setup: one sandbox at a
time, cold booted.

## Where this fits

SOMA is three layers, and this document is only the first one:

1. **`soma-vmm`**, the engine that runs one virtual machine per sandbox. This document sets that up.
2. **`soma-hostd`**, per-host pools of prepared workers that let one host serve many sandboxes
   quickly instead of cold booting each. Built and component-tested, not yet wired to the live
   command-line path.
3. **The fleet control plane**, which places and admits sandboxes across many hosts. Designed.

So this runbook stands up the floor. A production host is not just a machine that booted a
sandbox once: it is a **certified host profile**, an exact combination of host class, operating
system, kernel, CPU model, storage mode, network mode, and SOMA build that has passed the
conformance, isolation, cleanup, and performance gates. Changing any of those requires a new
certification. The support levels and the full prerequisite contract are in
[deployment portability](deployment-portability.md).

## Step 0: the server must be able to host a virtual machine

SOMA runs a real virtual machine per sandbox, so the host has to be able to do that. The engine
requires all of the following, and refuses to admit a workload without them:

- **Linux on x86_64.** Ubuntu 24.04 is the tested target.
- **KVM** and the required CPU virtualization features. On bare metal this is normal; on a cloud
  VM it requires nested virtualization enabled by the provider.
- **cgroup v2** and the namespace, seccomp, and resource-accounting controls the jail needs.
- **A private networking path** with enforceable denied and allowed policy.
- **A private writable-root mechanism** with verifiable cleanup, and a filesystem and kernel the
  selected Generation accepts.
- **Stable monotonic timing** and reserved capacity for the declared performance class.
- **sudo** to install packages and provision storage, and **network egress** to GitHub and a
  container registry.

The preflight that checks these is part of SOMA itself, so after Step 2 you run it rather than
checking by hand:

```sh
soma doctor --strict
```

A passing strict preflight means the host exposes the prerequisites this SOMA release checks. It
is not the full certification suite, but it is the gate to clear before running anything.

**Recommended: an XFS filesystem with reflink** (`xfs_info <mountpoint> | grep reflink=1`).
Writable sandbox disks are then near free to create, which is what makes running many sandboxes
cheap. Without it each sandbox copies its disk instead, which still works but costs time and space.

Every step below is a script in `scripts/`, so the basics cannot be forgotten. Run them in order.

## Step 1: prepare the host

```sh
./scripts/setup-host.sh
```

This installs every host dependency (including the ones that are easy to forget, `curl`,
`python3`, `ca-certificates`), adds you to the `kvm` and `docker` groups, installs the pinned Rust
toolchain and the `x86_64-unknown-linux-musl` target the guest agent needs, and prints a readiness
report. Open a new shell afterwards so the group changes take effect.

Getting the code onto the host is the one step no committed script can do for you, because this is
a private repository and a script must never carry a credential. Clone it with your own GitHub
access, or on a host without one transfer a `git bundle` made where you do have access:

```sh
# where you have access:   git bundle create soma.bundle origin/main
# on the host:             git clone soma.bundle SOMA
```

## Step 2: build SOMA

```sh
./scripts/build-soma.sh
```

This builds the three artifacts, the `soma` command line, the static guest agent, and the pinned
guest kernel (about 60 seconds of compile), and prints the path and digest of each.

## Step 3: build the pinned filesystem tools

```sh
./scripts/build-fs-tools.sh /srv/soma/fs-tools
```

The Generation compiler pins the exact tools it will accept, erofs-utils 1.9.4 and e2fsprogs
1.47.0, and rejects any other version. A distribution erofs-utils is almost never 1.9.4, so this
builds it from the pinned source in a container. On Ubuntu 24.04 the distribution e2fsprogs is
already 1.47.0, so it is used in place.

## Step 4: build your first templates (Generations)

A **Generation** is the thing a sandbox boots from: an OCI image compiled together with the kernel
and the guest agent into one content addressed, bootable image. You build it once, ahead of any
request, and it lives in a **prepared store** the KVM backend reads.

Prepare two: an empty sandbox with nothing installed but a shell, and the standard template, which
is node. A user can create a fresh empty sandbox and start from a blank slate, or take the standard
node template and have a runtime waiting. Each is one command, and each is a template you boot from
over and over:

```sh
# the empty sandbox: busybox, a shell and nothing else, a blank slate
./scripts/prepare-generation.sh busybox:stable-musl /srv/soma/prepared /srv/soma/fs-tools

# the standard template: node, a runtime ready to use
./scripts/prepare-generation.sh node:22 /srv/soma/prepared /srv/soma/fs-tools
```

Each call exports the image to an OCI layout with skopeo (which produces a real OCI layout
regardless of the local docker image store), compiles the Generation, and writes one prepared-store
entry. A prepared store is one directory per Generation, each holding the published Candidate
bytes, its artifact `store/`, and a `reference` file naming the image. Prepare as many templates as
you want to offer, once, ahead of demand.

## Step 5: run your first sandboxes

Point the writable disks at the reflink filesystem, then boot each template and run a command
inside a fresh VM. Start with the empty one, then the runtime:

```sh
export SOMA_GENERATION_STORE=/srv/soma/prepared
export SOMA_HEAD_DIR=/srv/soma/heads              # on the XFS reflink volume
export SOMA_ALLOW_UNCERTIFIED_GENERATION=1        # see the note below

# the empty sandbox: prove it boots and runs a command
./target/release/soma --backend kvm run busybox:stable-musl -- /bin/busybox uname -a

# the node sandbox: a real runtime, running inside the VM
./target/release/soma --backend kvm run node:22 -- /usr/local/bin/node --version
```

The node sandbox prints `v22.23.2`, from the interpreter running inside the virtual machine, and
cleans the machine up afterwards. Each command proves the whole chain: a capable host, the built
software, a valid template, a booting KVM guest, a command executed inside it, and teardown. This
is the shape of the first-run experience: create a template, boot a sandbox from it, see it run.

The `SOMA_ALLOW_UNCERTIFIED_GENERATION` flag is required on purpose. Generation certification does
not exist yet, so every template a host can build today is an unverified Candidate. The backend
refuses to launch one unless this flag is set, so that unverified images cannot boot by accident.
Setting it is the explicit opt in for a development host.

## What this gives you, and what it does not

**What works today.** One hardware isolated sandbox at a time, cold booted, driven from the command
line, running your workload and cleaning up. This is a real development and evaluation capability on
a real KVM host.

**What a production sandbox service still needs, and is not built yet:**

- A daemon or API to call, rather than a one shot command. The backend currently tracks a single
  live sandbox.
- Many concurrent sandboxes with capacity admission, so one host can serve a fleet safely.
- A jail around the virtual machine process. Today the VM runs inside the command line process
  rather than as a separately confined `soma-vmm` process. This is the most important gap before
  anything faces untrusted users.
- Prepared restore, which reaches a ready sandbox in milliseconds instead of the current cold boot.
- Certified Generations, so a template is verified before it can launch rather than opted into.

The full path from this development setup to a production admitted service is tracked in
[the public KVM Backend audit](../reviews/2026-08-30-public-kvm-backend-audit.md) and the ticket
map in [the VMM decision map](../research/vmm-decision-map.md).

## Teardown

Stop any running sandbox with the command line, remove the prepared store and head directories
under `/srv/soma`, and if a tunnel was opened to reach the host, close it. Rotate any credential
that was shared to gain access.
