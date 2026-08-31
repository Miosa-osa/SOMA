# The end to end check

`scripts/end-to-end-check.sh` walks the whole documented flow on one host, from a bare checkout to
a sandbox that ran a command and left nothing behind, and names the first stage that broke.

The repository already has unit tests, component tests, and live KVM tests. Each of them proves a
part. None of them proves the join, and the join is where a host stops working: a kernel that
builds but does not boot, a Generation that compiles but cannot be resolved, a run that succeeds
and leaves an overlay head on the disk. This check exists to fail loudly at whichever step breaks.

It is a flow check, not a benchmark and not a certification. Read [what it does not prove](#what-it-does-not-prove)
before quoting it for anything.

## What it proves

Six stages, in order. Each records `passed`, `failed`, or `skipped` with a bounded detail string.
Once a stage fails, every later stage is recorded `skipped` rather than attempted, so a break is
never hidden by a cascade of secondary failures.

| Stage | What runs | What has to be true |
| --- | --- | --- |
| `host_setup` | the readiness half of `scripts/setup-host.sh`, and `scripts/build-fs-tools.sh` under `--with-host-setup` | `/dev/kvm` is readable and writable, the CPU exposes `vmx` or `svm`, cgroup v2 is mounted, skopeo, cargo, python3 and docker are present, the musl target is installed, and the `erofs/` and `e2fsprogs/` directories of pinned tools are populated |
| `build` | `scripts/build-soma.sh` | the command line, the static guest agent, and the pinned PVH kernel are all present, and their digests are recorded |
| `prepare_generation` | `scripts/prepare-generation.sh` | one OCI image is exported and compiled into one prepared entry that carries a Candidate, an artifact store, and the reference it was prepared for |
| `capture_snapshot` | `crates/soma-local/examples/capture_snapshot.rs` | the entry gains `snapshot/state.somasnap` and a sterile `snapshot/overlay.raw`, so later launches restore instead of cold booting |
| `run_sandbox` | `soma --backend kvm run ... --format json`, twice | each envelope reports `status: ok`, the guest command exits zero, the guest's own stdout matches the expected pattern, and the receipt reports every owned resource released |
| `cleanup_proof` | host probes taken either side of the runs | no overlay head, no process, no network namespace, no nftables table, and no state root byte outlived the sandbox |

The run stage runs twice on purpose. The first run creates the durable state a fresh state root
has never held; the second proves a steady state run adds nothing to it. One run cannot separate
those two, so one run cannot support a no growth claim.

### The cleanup checks

These are the checks the image matrix did not make, and they are where a real leak hides. Each is
recorded separately in `cleanup-checks.tsv` as `clean`, `leaked`, or `unverified`.

- **`overlay_heads`**: the head directory must hold no entry. The KVM backend creates each private
  head and unlinks its name immediately, keeping only the descriptor, so a named file surviving the
  run means an Instance disk outlived its Instance.
- **`processes`**: no process whose argument vector names this run's state root may be alive
  afterwards. Matching on the state root rather than on the word `soma` keeps another agent's
  concurrent run on the same host from reading as this run's orphan.
- **`netns`**: the set of network namespaces must be the same before and after.
- **`nftables`**: the set of nftables tables must be the same before and after. This one is
  recorded `unverified` rather than passed when `nft` cannot be read.
- **`state_root`**: `du -sb` on the state root must be byte identical across the second run.

The stage fails only when a check observed a difference. An `unverified` check is printed in the
summary and carried into the results file rather than being counted as a pass.

## What it does not prove

- **It is one host, one image, one shape, one command, one moment.** It is not a benchmark. The
  millisecond figures in the run detail come from the receipt and are single samples on whatever
  else that host was doing; do not quote them.
- **It launches an uncertified Candidate.** Certification does not exist, so the run stage sets
  `SOMA_ALLOW_UNCERTIFIED_GENERATION=1`, exactly as the development path documented in
  [the server setup guide](../operations/server-setup.md) does. It proves nothing about a
  certified Generation, and nothing about promotion.
- **It does not prove the jail.** The virtual machine runs inside the command line process; the
  real `soma-vmm` under `soma-jail` is not on this path.
- **It does not prove the network.** The one guest device is link down, egress is denied, and the
  receipt reports the network as `not_owned`. The namespace and nftables checks therefore prove
  that a run which owns no network created none, not that a network attach cleans up.
- **It does not prove concurrency.** Two sequential runs say nothing about a burst.
- **The `netns` and `nftables` checks compare a global host surface.** Another agent changing
  either while the check runs shows up as a difference. The check prints exactly which entry
  appeared, so a false positive is inspectable rather than mysterious.
- **`host_setup` verifies readiness, it does not install.** Without `--with-host-setup` it reads
  the host rather than changing it, which is what makes it safe to run on a shared machine.
- **A passing check does not admit anything to production.** It is a flow check. The gates a
  production claim needs are in [the engineering standard](../standards/sota-engineering-standard.md).

## How to run it

On a prepared Ubuntu x86_64 KVM host, from the repository root:

```sh
scripts/end-to-end-check.sh \
    --work /srv/soma/e2e-run \
    --fs-tools /srv/soma/fs-tools
```

It writes nothing outside `--work` and the repository's own `target/` and `kernel/out/`. The work
directory holds its own prepared store, head directory, and state root, so it never touches a
shared store and the cleanup proof can be exact. On a shared host, give `--work` a name of your own.

On a host that has never been prepared, add `--with-host-setup` to run `setup-host.sh` and
`build-fs-tools.sh` first. Both need sudo and both change the machine, which is why they are opt in.

| Option | Default | Meaning |
| --- | --- | --- |
| `--image REF` | `node:22` | the OCI image to compile and launch |
| `--work DIR` | a fresh temporary directory | where the store, heads, state root, logs, and results go |
| `--fs-tools DIR` | `./fs-tools` | the pinned erofs and e2fsprogs tools from `build-fs-tools.sh` |
| `--expect REGEX` | `^v[0-9]` | the pattern the guest's stdout must match |
| `--command CMD ARG...` | `/usr/local/bin/node --version` | the command to run inside the sandbox; must come last |
| `--with-host-setup` | off | run `setup-host.sh` and `build-fs-tools.sh` before checking |
| `--purge` | off | delete the store, heads, and state root after reporting, keeping results and logs |

Set `SOMA_E2E_COMMIT` when running from a checkout that carries no `.git`, so the results name the
commit rather than `unknown`.

A different image needs its own command and pattern:

```sh
scripts/end-to-end-check.sh --image alpine:3.20 --expect 'Alpine Linux' \
    --command /bin/cat /etc/os-release
```

The exit status is `0` when every stage passed, `1` when one failed, and `2` on a usage or platform
error.

## What it writes

Inside the work directory:

- `results.json`, schema `soma.e2e.v1`: the image, host, commit, the first failing stage or `null`,
  and one object per stage with its status, bounded detail, duration, and log path.
- `cleanup-checks.tsv`: one row per cleanup check, as name, verdict, and detail.
- `logs/<stage>.log`: the full untruncated output of each stage. The detail string is bounded to
  200 characters so a summary stays readable; the log is where the whole failure lives.
- `run-first.json` and `run-second.json`: the two machine envelopes, kept so a receipt can be
  re-read without another run.

The summary prints the same table to the terminal and, on failure, names the stage and the log to
open.

## Reading a failure

The point of the check is the failing stage, so read it in this order:

1. The summary line names the stage and carries the bounded reason.
2. `logs/<stage>.log` carries everything that stage printed.
3. Every stage after it says `not attempted after <stage> failed`. That is not extra information:
   those stages were never run, and nothing about them was proved either way.

A `run_sandbox` failure quotes the exact judgement from `scripts/end-to-end/inspect-run.py`: which
of the envelope status, the exit code, the guest output, or the receipt's cleanup dispositions was
wrong. A `cleanup_proof` failure names each leaked check and what appeared.

## Where it fits

[The server setup guide](../operations/server-setup.md) is the prose version of the same flow, for
a person setting up a host by hand. This check is the executable version, for proving that the flow
still works after a change. [The engineering standard](../standards/sota-engineering-standard.md)
defines the status vocabulary a result may be described in, and
[the claim ledger](../claim-ledger.md) is where a claim backed by a run belongs.
