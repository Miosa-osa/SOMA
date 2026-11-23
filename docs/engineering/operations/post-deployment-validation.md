# Post-deployment host validation runbook

## Intended executor

This runbook is the handoff for the agent that acts after the SOMA change is merged and deployed to a test host.
The executor must record evidence in [validation-report-template.md](validation-report-template.md) and must stop at every failed hard gate.
Passing platform-neutral tests on Apple Silicon macOS is useful development evidence, but it is not host acceptance.

SOMA is pre-alpha and is not safe for untrusted production workloads.
Use an isolated non-production host, synthetic data, and a workload with no operator or tenant credentials.

## Required outcome

The executor must determine which of the following evidence levels the deployed revision actually earns:

1. The release identity and Linux host are reproducible and correctly targeted.
2. The deployed revision can access the required Ubuntu 24.04 x86_64 KVM interfaces.
3. The implemented portion of the planned public `Launch`, `Execute`, and `Stop` contract passes semantic-interface tests.
4. A certified Generation can restore with private copy-on-write memory and disk when that path exists.
5. One restored Instance reaches Ready only after authenticated Repair and a no-op Execute probe when that path exists.
6. Concurrent restores preserve identity, isolation, cleanup, and success invariants when that path exists.
7. An out-of-tree provider adapter can run the exact upstream ComputeSDK Burst TTI benchmark when the complete provider path exists.

Do not claim a later evidence level because an earlier scaffold, mock, capability probe, or milestone passed.
If the deployed revision implements only the pre-alpha contract slice and KVM probe, report exactly that and mark the restore and benchmark phases blocked by missing implementation.

## Inputs required from the merge and deployment agent

- The exact `Miosa-osa/SOMA` merge commit SHA.
- The immutable release artifact identity and its SHA-256 digest.
- The deployment identifier, target host identifier, and deployment timestamp.
- The absolute installed path of `soma-vmm` or the exact command used to invoke the deployed artifact.
- The host artifact and disk-head filesystem paths, supplied without secrets.
- The Generation identity and certification evidence if Generation restore is implemented.
- The operator-side adapter location if provider or ComputeSDK validation is implemented.
- The rollback owner and the previous known-good deployment identity.

Do not continue if the deployed bytes cannot be tied to the merge commit and immutable digest.

## Phase 0: Preserve the measurement boundary

Clone or fetch `Miosa-osa/SOMA` at the exact merge commit in an isolated validation directory.
Do not validate an uncommitted working tree or rebuild from a moving branch.
Do not modify or commit to the upstream ComputeSDK SDK or benchmark repositories.
Any temporary provider integration must live in an out-of-tree adapter and record its own commit and diff.

Record the following before changing host state:

```sh
git rev-parse HEAD
git status --short
sha256sum /absolute/path/to/soma-vmm
```

The worktree must be clean and the checksum must match the deployed artifact manifest.
If no `soma-vmm` binary exists in the deployed revision, record that fact and continue only with checks that revision actually supports.

## Phase 1: Qualify the Linux KVM host

The production target for this validation is Ubuntu 24.04 on x86_64 bare metal with KVM.
A nested VM result, another distribution, another Ubuntu release, or another architecture is diagnostic evidence rather than target certification.

Capture the host identity with read-only commands:

```sh
cat /etc/os-release
uname -a
uname -m
lscpu
cat /proc/cmdline
test -c /dev/kvm
stat /dev/kvm
test -r /dev/kvm
test -w /dev/kvm
test -c /dev/net/tun
stat /dev/net/tun
findmnt -t cgroup2
```

The target gate requires all of the following:

- `/etc/os-release` identifies Ubuntu 24.04.
- `uname -m` reports `x86_64`.
- The CPU exposes `vmx` or `svm` and KVM is available to the deployment identity.
- `/dev/kvm` is a character device readable and writable by the exact account that launches SOMA.
- Cgroup v2 is mounted.
- `/dev/net/tun` is available for the planned TAP path.
- The host is identified as bare metal or the report clearly labels nested virtualization.

Record the host kernel package, CPU model, microcode, NUMA topology, physical RAM, and active mitigations.
Do not disable mitigations to improve a benchmark without a separate security decision and a separately labeled result.

## Phase 2: Qualify storage and artifact immutability

Run these read-only checks against the actual artifact store and private disk-head directories supplied by deployment:

```sh
findmnt -T /absolute/path/to/artifact-store
findmnt -T /absolute/path/to/disk-head-directory
stat -f /absolute/path/to/artifact-store
stat -f /absolute/path/to/disk-head-directory
```

For the initial MIOSA target, capture `xfs_info` for the disk-head filesystem and verify `reflink=1`.
Do not assume that a successful file copy proves reflink behavior.
Use the repository's non-destructive reflink test when one exists and record inode, extent, and isolation evidence.

Verify that certified Generation Artifacts cannot be replaced or mutated by the `soma-vmm` process identity.
Verify that every Instance receives a distinct writable disk head.
Do not continue to real restore if the memory Artifact is writable by the VMM or if the filesystem cannot provide the required private copy-on-write contract.

## Phase 3: Verify the deployed revision

Run repository checks from the exact merge commit on the target host:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Run the repository's Linux KVM test or capability probe using the deployment account.
Record the exact command and complete output rather than substituting an ad hoc ioctl test.
If the repository does not yet provide a runnable probe, mark this phase blocked by missing implementation.

For the Phase 0 workspace, run the semantic contract tests and the explicitly ignored host test separately:

```sh
cargo test -p soma-vmm
cargo test -p soma-kvm --test kvm_probe -- --ignored
```

The second command must run on the target Ubuntu 24.04 x86_64 host with accessible `/dev/kvm`.
Its explicit ignored status prevents an ordinary macOS or unprivileged test run from being mistaken for KVM evidence.

The minimum pre-alpha pass requires the following:

- The workspace builds with locked dependencies on Ubuntu 24.04 x86_64.
- Formatting, lint, and platform-neutral tests pass.
- Linux-only KVM tests run rather than skip.
- The capability probe opens KVM, verifies the KVM interface version, and creates then closes a test VM without leaking resources when that behavior is implemented.
- Tests drive every implemented command through the semantic public interface rather than private lifecycle calls.
- Encoded-transport conformance is required only after the deployed revision contains a protocol codec.

Retain test logs and the target directory metadata needed to reproduce the build.

## Phase 4: Validate one real Generation restore

This phase begins only when the deployed revision contains a real Generation builder or consumes a certified Generation produced by an approved external builder.
A mock backend, synthetic receipt, or state-machine test cannot pass this phase.

Use an OCI-derived `node:22` Generation with one vCPU and 1024 MiB of memory unless the supported profile declares a different fixed requirement.
Record the OCI digest, guest kernel digest, root filesystem digest, memory Artifact digest, machine-state digest, guest-agent identity, compatibility fingerprint, and certification evidence.
Keep registry acquisition and Generation construction outside the warm Launch timer.

Launch one Machine through the public interface and retain every milestone plus the terminal receipt.
The required order is:

```text
REQUEST_ACCEPTED
RESOURCES_OWNED
PROCESS_STARTED
ARTIFACTS_VERIFIED
MEMORY_MAPPED
KVM_CREATED
KVM_STATE_RESTORED
VCPU_RESUMED
AGENT_AUTHENTICATED
GENERATION_ACKNOWLEDGED
IDENTITY_REPAIRED
NETWORK_REPAIRED
FIRST_COMMAND_SUCCEEDED
READY
```

Verify that the first internal readiness probe is an authenticated no-op Execute operation after Repair.
Then execute `node -v` through the same public Execute path and record authenticated terminal evidence.
Finally call Stop and retain its terminal receipt.
During the bounded shutdown handshake, replay the same Stop operation and verify that the recorded outcome is returned without touching an unrelated resource.
After the VMM exits, verify that the operator-retained receipt resolves the outcome without launching a replacement process.

The phase fails if any intermediate milestone is reported as Ready, any identity remains cloned, any command path bypasses guest authentication, or cleanup leaves owned resources behind.

## Phase 5: Prove failure behavior

Run each supported failure test through the public interface and retain the typed fault and cleanup evidence.
At minimum, test these cases when the corresponding implementation exists:

- A changed, truncated, substituted, or writable memory Artifact.
- An incompatible architecture, CPU feature class, runtime version, device layout, or guest-agent contract.
- A disk filesystem without the required private copy-on-write guarantee.
- A replayed guest-agent handshake or receipt from another Instance.
- A reused `OperationId` with a structurally different request in Phase 0 or a different canonical fingerprint after the wire protocol exists.
- Guest-agent authentication failure.
- Failure during identity, entropy, time, network, or transport Repair.
- No-op Execute failure after Repair.
- VMM exit, caller timeout, and parent death during each owned-resource phase.
- Stop repeated after partial and complete cleanup.

There must be no cold-boot, alternate-VMM, compatibility downgrade, or unauthenticated Ready fallback.
The ownership evidence must prove that cleanup targets only resources bound to the matching receipt.

## Phase 6: Prove clone isolation and burst behavior

Restore at least ten Instances sequentially from one Generation and compare their repaired identities.
Then restore 100 Instances concurrently on a host sized for the declared profile.
Every Instance must use the same immutable Generation Artifacts while owning independent mutable memory, disk, network, process, and control state.

Verify all of the following:

- A write to guest memory in one Instance is invisible to every other Instance.
- A disk write in one Instance is invisible to every other disk head and the immutable base.
- Machine identity, hostname, MAC, IP, vsock generation, and authenticated session identity are unique.
- Replayed control messages and receipts fail across Instances.
- Every successful Launch reaches Ready only after Repair and a no-op Execute probe.
- Every Instance successfully runs `node -v`.
- Every Stop succeeds and all owned cgroups, namespaces, TAPs, disk heads, processes, mappings, and sockets are accounted for.

Record every failure rather than dropping it from the cohort.
Record whether the host page cache and prepared resources were cold, warm, or preclaimed.
Do not merge results from different preparation classes.

## Phase 7: Run the exact ComputeSDK benchmark

This phase begins only after the complete provider create, Execute, and destroy path exists outside the upstream ComputeSDK repository.
Use the authoritative upstream benchmark at a recorded commit without modifying its timing boundary or failure accounting.

The required profile is:

- OCI image `node:22`.
- 100 iterations.
- Concurrency 100 opened as one burst.
- Timer starts before the provider create call.
- Timer stops only after `node -v` succeeds inside the created sandbox.
- Destroy is outside TTI but must run and its result must be retained.

Retain all raw samples, errors, cleanup results, wall time, median, p95, p99, and success rate.
Record the benchmark runner location, network path, SOMA commit, adapter commit, host data, Generation identity, cache state, and all preparation outside the timer.
Do not compare an already-Ready lease or paused-Machine lease with another provider's on-demand restore without labeling both paths.

The engineering target is a median below 50 ms, p99 below 90 ms, and 100 successful launches, commands, and cleanups out of 100.
These values remain targets until a complete reproducible run produces evidence.

The authoritative 100-sample cohort is one externally comparable result, not sufficient tail-engineering evidence by itself.
Before publishing a stable performance claim, run at least 100 identical bursts for at least 10,000 retained samples and report per-burst maxima.

## Phase 8: Publish the validation result

Complete [validation-report-template.md](validation-report-template.md) with links to immutable logs and raw samples.
State the highest completed evidence level and list every blocked, skipped, failed, or unsupported phase.
Separate facts, measurements, targets, and inferences.
Do not describe a platform-neutral mock, Apple Silicon run, or KVM capability probe as a working microVM restore.

A full target-host acceptance requires all applicable hard gates, zero unexplained resource leaks, and reproducible evidence from the exact deployed commit.
Any failed security invariant blocks untrusted execution regardless of benchmark speed.

## Containment and rollback

If a security, isolation, identity, cleanup, or compatibility invariant fails, stop admitting new test Launch requests immediately.
Use the public Stop operation and deployment ownership records to terminate only Instances owned by the failed validation.
Do not delete Generation Artifacts while any VMM may retain an open handle or mapping.
Preserve logs, receipts, manifests, checksums, and host metadata before rolling the deployment back.

Restore the previous known-good deployment through the operator's documented deployment process.
Repeat Phases 0 through 3 against the rollback identity before declaring the host stable.
Because SOMA is pre-alpha, rollback success does not authorize untrusted production use.
