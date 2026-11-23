# One Apple backend sandbox driven by five processes - 2026-11-20

## Evidence boundary

This run proves that SOMA revision `244e218edb07def23dfdbbff844a6440a8523d36` can use its Apple development backend on Apple Silicon macOS to run Node 22 and Ubuntu 24.04 one-shot commands with complete cleanup, and to drive one sandbox through five separate `soma` processes by its Instance identity alone.
It settles the run that [macOS was classified as hosting a machine it does not hold](2026-09-01-macos-hands-back-a-usable-identity.md) named as unproved.
It does not prove the custom SOMA VMM, an OCI-derived SOMA Generation, cryptographic guest identity, production isolation, snapshot restore, a prepared worker, or any latency target.

## Identities

- SOMA Git revision: `244e218edb07def23dfdbbff844a6440a8523d36`, `r3/soma-unified`.
- Host: Apple M3 Ultra, macOS 26.6.2 build 25G83, ARM64.
- Apple container CLI: `1.3.0`, release build, commit `d6de569`, at the pinned path `scripts/bootstrap-macos.sh` installs.
- Rust compiler: `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
- `node:22`: index `sha256:e13726ca604184d7a91fcacab1fc09eefd642dc28e79ac34e85540dd93b78cfb`, manifest `sha256:f796bda8a91392fe7b5c974ff42abaa5cc801eb6987dca967f1db8be493e0ee3`, `linux/arm64/v8`.
- `ubuntu:24.04`: index `sha256:9c7f200c07e3d4a03e7249d2c16bf66a51120fb876aa2242cf31451412543289`, manifest `sha256:cd04f18ea6639206fcecf4c148bde909cd08098e1d0569117c2af59242059558`, `linux/arm64/v8`.

## Invocation

The runtime was stopped before the run and was started with the repository's own bootstrap, which found the pinned runtime already installed and installed nothing.
Every command used a fresh state root and `--backend macos --format json`.

```sh
bash scripts/bootstrap-macos.sh
soma --backend macos doctor --strict
soma --backend macos run node:22 -- /usr/local/bin/node --version          # three times
soma --backend macos run ubuntu:24.04 -- /usr/bin/cat /etc/os-release
soma --backend macos machine launch --instance-id "$ID" ubuntu:24.04       # process 1
soma --backend macos machine exec --instance-id "$ID" -- /usr/bin/bash -c 'echo written-by-process-2 > /tmp/soma-proof'   # process 2
soma --backend macos machine exec --instance-id "$ID" -- /usr/bin/cat /tmp/soma-proof   # process 3
soma --backend macos machine inspect --instance-id "$ID"                   # process 4
soma --backend macos machine destroy --instance-id "$ID"                   # process 5
container system stop
```

Each command's full JSON envelope and receipt is retained in [the raw record](raw/2026-11-20-apple-backend-five-process-lifecycle/), with `exits.txt` holding every exit status.

## Result

The strict probe passed and reported the runtime ready but not production ready.
All three Node 22 runs exited 0 and returned exactly `v22.23.5\n`.
The Ubuntu run exited 0 and returned the `/etc/os-release` of Ubuntu 24.04.5 LTS.
Every one-shot receipt reported `hardware_virtual_machine` isolation, on-demand preparation, 1 effective vCPU and 1,024 MiB of memory, storage not verified, a detached network with egress and DNS denied, and complete machine, memory, storage, and runtime-attachment cleanup.

The five-process lifecycle passed end to end.
Process 1 launched the sandbox and returned it `ready`.
Process 2 wrote a file inside it, and process 3, which shared nothing with process 2 but the identity, read back exactly `written-by-process-2\n`.
Process 4 inspected it as `ready` on `macos_virtualization`, and process 5 destroyed it with complete cleanup.
After the run the runtime listed no `soma-` container, and after `container system stop` it reported `unregistered`, the state it was in before the run.

| Run | Accepted to cleanup finished |
| --- | ---: |
| Node 22, first | 6,592.9 ms |
| Node 22, second | 8,330.3 ms |
| Node 22, third | 3,843.7 ms |
| Ubuntu 24.04 | 4,272.2 ms |
| Five-process launch, accepted to ready | 1,883.8 ms |

## Measurement warning

These are development-backend diagnostics, not benchmarks.
The host was shared with several concurrent Rust builds, at a load average of about 20 to 30 on 28 cores, which is why the same command varied from 3.8 s to 8.3 s, and why the figures are slower than the 1.995 s of [the August one-shot](2026-08-29-apple-node22-one-shot.md).
The run before this one, at `9eadb60` on a feature branch, timed out at the 30 s default deadline on the first command for each image, because the runtime was pulling a newer upstream digest of `node:22` and `ubuntu:24.04`; pulling `node:22` alone took about 25 s.
The default deadline therefore covers a cold image pull only when the pull is short.
