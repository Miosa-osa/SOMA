# x86_64 capture and restore on the per-Instance authority design - 2026-08-30

## Status: current

This is the recapture that finding P1.5 of [the second implementation re-audit](../reviews/2026-08-29-implementation-reaudit.md) required.
It replaces [the 2026-08-29 run](2026-08-29-x86_64-snapshot-restore.md), which remains retained and is labeled historical because it was captured at `7c1127d`, before ADR 0024 moved the guest responder secret out of the Generation.

The earlier run could not certify current bytes. This one was made at the current commit with fresh per-Instance authority.

## Run identity

| | |
| --- | --- |
| Commit | `5d71524e0a602d6e5e852ced1e70d87e35b56e2c` |
| Kernel | `vmlinux-6.12.107-soma-v1`, sha256 `f1af3a142fa39916cfac425a01b16b5f328279823533421c9eec3f192c05b746` |
| Guest agent | sha256 `b687587abff9614b502044a5bc260c5d401604f0e6f34fe8192ef60a47644cbd`, `x86_64-unknown-linux-musl`, release |
| Host | Linux 7.0.0-30-generic x86_64, Ubuntu 24.04.4 LTS |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18); erofs-utils 1.9.4 (pinned) |
| Test build profile | **debug**, in a container with `--device /dev/kvm` and `--test-threads=1` |
| Suite | `cargo test -p soma-kvm --test x86_64_snapshot_restore -- --ignored --test-threads=1 --nocapture` |
| Result | 6 passed, 0 failed, 403.31 s |

## Generation and snapshot identities

| Object | Digest | Size |
| --- | --- | ---: |
| Generation | `sha256:3648c65297efe4145ff20319b314ddb4765841a8d0b70c101a99dfd1e92265a1` | |
| EROFS root | `sha256:48a6cf92bd0b4a57ee7ea87f0d3efe774ad26bd47d6db4ed6c23c83dcfe8aa48` | 1,129,172,992 |
| Overlay template | `sha256:ecfecc597f7dfa7b98dec28adb5eeb3a15357e090cbadf62fb1c627dc41fb790` | 268,435,456 |
| Initramfs | `sha256:f5946942bde09e5ca31e7138000cd26e1b87e24e6597eb46ad495af7e0906681` | |
| `memory.raw` | `sha256:e6212282f4d62508041947bdfa9f98d2d2d84819e8239dbb57925e7be7d2bafa` | 1,073,741,824 |
| `overlay.raw` | `sha256:b0d712e5ed87a6f827b7b81d289cc8381cac16b1db75af368906778722e82af5` | 268,435,456 |
| `state.somasnap` | `sha256:a200ee3f0aeab83a46ecae8498a3786d5326fa1d6bace0227471e12e831606cb` | 9,566 |

## What the run proves

### No reusable private authority survives capture

This is the finding's central requirement, and it is proved by an object scan over the published artifacts rather than by assertion:

```
[scan] the launch-page domain occurs 2 times in memory.raw and 1 times in the pinned
       agent binary; none of them decodes as a launch page
[scan] no Instance responder identity appears in memory.raw, overlay.raw, or
       state.somasnap; capture precedes every launch page
[scan] memory.raw is 1073741824 bytes covering [0, 0x40000000); the launch page slot
       is at 0xd0100000
```

Two properties matter here and are separately established.

The launch-page domain string does occur twice in guest RAM, and that has an innocent explanation the test forces rather than assumes: the pinned agent binary contains the constant it compares against, and the scan asserts that count is non-zero so the occurrences in RAM are not unexplained. Every occurrence is then fed to the production decoder, and none of them decodes as a launch page.

Separately, a freshly generated Instance's responder secret halves appear nowhere in any published object. The launch page slot sits at `0xd0100000`, outside the `[0, 0x40000000)` range `memory.raw` covers, so the captured memory object cannot contain it by construction; capture happens at the disconnected repair point, before any launch material exists.

### The snapshot is refused when it is not exactly what it claims

Three rejections, each before any vCPU exists:

```
[tamper] state.somasnap -> section role 0x0002 digest mismatch
[tamper] memory.raw     -> memory digest sha256:6a63eaca... does not match sha256:e6212282...
[compatibility] cpu template -> CpuTemplate { expected: sha256:214170df..., actual: sha256:204170df... }
```

The CPU-template rejection is a single flipped nibble, and it is refused.

### Two restores of one snapshot are independent

`two_restores_of_one_snapshot_are_independent_instances` passed: each clone saw its own private write, neither saw the other's, and the shared memory object was unchanged under the private mapping.

## Timings

These are **debug-build** figures and are not comparable with the release-build numbers in [the warm-path evidence](2026-08-30-warm-path-optimization.md). They are recorded because the run produced them, not as a performance claim.

Warm restore to `Ready` over ten iterations, in nanoseconds:

```
n=10  p50=16,279,321  p99=19,081,865  min=15,329,087  max=19,081,865
samples=[15329087, 15351652, 16216465, 16267003, 16279321,
         16280217, 16515582, 16843096, 18678956, 19081865]
```

One restore's stage timeline, nanoseconds since the restore began:

```
validate manifest              515,414
create VM                    1,516,711
map memory privately         1,536,736
launch page slot mapped      1,808,701
register memory slots        2,043,923
irqchip, PIT, routes         2,440,386
devices restored             2,496,349
vCPU created                 4,270,266
vCPU state restored          4,441,711
eventfds and interrupt state 4,482,098
fresh launch page written    16,786,752
device thread serving        16,890,230
resume                       17,232,252
launch page consumed         18,322,985
vsock connected              19,087,274
handshake done               26,741,227
repair done                  27,560,691
ready                        29,952,247
execute done                 40,729,379
```

With ten samples the nearest-rank p99 is the largest sample rather than an interior order statistic.

## What this does not prove

- It does not measure the release build, and says nothing about the 11.70 ms figure recorded elsewhere.
- It is one host, one day, one machine shape, one Generation, `--test-threads=1`, with a warm page cache. It says nothing about concurrent restores.
- The scan proves the absence of the Instance responder identity and of any decodable launch page in the published objects. It is not a general proof that the objects contain no secret of any other kind.
- It is not a certified budget, a latency objective, or a statement about production admission.
- Guest networking, Generation certification, and a jail around the real `soma-vmm` remain capability gaps recorded in [the claim ledger](../claim-ledger.md); nothing here changes their status.
