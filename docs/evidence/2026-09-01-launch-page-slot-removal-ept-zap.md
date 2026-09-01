# Why a restored resume takes more EPT violations than it touches pages - 2026-09-01

## Capability status: Live-proved at `9a38946`, diagnostic split with a shipped-default lever

[The resume page-in record](2026-08-31-restore-resume-page-in.md) established that the `ready`
segment's largest mechanical cost is the EPT violations a restored guest takes, and closed with a
mechanism: the cost "is proportional to the number of distinct guest pages the resume touches and
to nothing else". This record measures which pages those are, and finds that sentence is wrong in
a way that hides a lever.

A resume takes **1.85 EPT violations per distinct guest page**, not one. Nearly half of them are
repeat violations on pages already faulted in, and most of those repeats have one cause: SOMA
removes the launch page's KVM memory slot while the guest is still running, KVM invalidates that
VM's whole extended page table when a slot is deleted, and the guest re-faults its live working
set seven milliseconds before it reports Ready.

**No number in the attribution sections is a latency result.** The cohort figures in the last two
sections are receipt milestones from matched cohorts and may be compared with each other, and with
nothing else.

## Observation identity

| Field | Observed value |
|---|---|
| Host | eval-1, one bare-metal Ubuntu host |
| CPU | Intel Xeon Gold 6138 at 2.00 GHz, 80 logical CPUs |
| Host kernel | Linux 6.8.0-138 |
| Guest kernel | 6.12.107-soma-v1, symbols from the same `vmlinux` the Generation was built with |
| Storage | XFS on `/srv` with `reflink=1` |
| SOMA revision | `03039729` with this branch's two-file change applied. Both arms of every comparison are that one binary |
| Generations | `/srv/soma/sweep/{node-1024-10240,bb-1024-10240}`, the same ones the page-in record used |
| Path | Prepared restore, one process per sandbox, the development command line |
| Host load | Other agents were working on this host throughout. Every cohort records the busy fraction of `/proc/stat` across its own window, and one cohort was discarded on it |

The two changed files are byte-identical between `03039729` and `origin/main`, so the change
measured here and the change committed on this branch are the same change.

## The instrument

`kvm:kvm_page_fault` carries the guest physical address, the guest instruction pointer, and the
access class of every EPT violation, and on this kernel it fires exactly once per violation:
over one resume it counted 2913 events against 2913 `EPT_VIOLATION` exits. The window is the
machine's own `RunStart` to `Ready`, read from the timeline the same launch wrote, so the
recording overhead lengthening a resume cannot silently shorten the window it is measured over.

`kvm:kvm_try_async_get_page` and the `kvm_async_pf_*` tracepoints never fire, so asynchronous
page faults are not part of this and were eliminated as an explanation before any other was tried.

## What the resume touches

One `node:22` restore at 1024 MiB, two consecutive launches, both untouched by any change.

| Quantity | run 2 | run 3 |
| --- | ---: | ---: |
| EPT violations, `RunStart` to `Ready` | 3217 | 3219 |
| Distinct guest pages faulted | 1743 | 1743 |
| **Violations per distinct page** | **1.85** | **1.85** |
| Pages faulted once / twice / three or more | 755/645/343 | 751/649/343 |
| Pages read only / written only / read then written | 1078/340/325 | 1079/337/327 |
| Distinct faulting guest instruction pointers | 1686 | 1705 |

Two facts sit in that table. The first is that the resume's working set is **1743 pages, 6.8 MiB**,
and that the page-in record's 3199 was never a page count. The second is that only 325 pages are
read and then written, so copy-on-write promotion accounts for at most a fifth of the 1476 repeat
violations; the rest are the same page faulted again with the same access class, which can only
happen if its extended-page-table entry was thrown away in between.

Where the faults land, and who takes them:

| Bucket | Share |
| --- | ---: |
| Guest physical 0 to 64 MiB | 84.2% |
| Guest physical 960 to 1024 MiB | 14.8% |
| Taken from guest kernel code | 92.0% |
| Taken from guest user code | 8.0% |

The faulting code is a flat tail: 1705 distinct instruction pointers over 3219 faults, headed by
`clear_page_orig` at 7.0%, `__d_rehash` at 3.4% and `memset_orig` at 2.4%. That shape is what a
working set being re-established through an empty extended page table looks like, and it is why
"the guest touches pages it need not" is not the lever here. The lever is the repeats.

## What throws the extended page table away

`kvmmmu:kvm_mmu_zap_all_fast` fires once during the resume, at **24.73 ms** after `RunStart`, on
the sandbox thread rather than the vCPU thread, followed immediately by 102 `prepare_zap_page`
events. The machine's own `LaunchPageRetired` milestone is at 26.22 ms. The violation rate over
the same resume runs at 50 to 70 per millisecond until then and reaches **247 per millisecond** in
the millisecond the milestone lands in, staying above 100 until Ready.

Splitting one resume at that boundary:

| Window | Violations | Distinct pages | Per page |
| --- | ---: | ---: | ---: |
| `RunStart` to `LaunchPageRetired` | 2029 | 1420 | 1.43 |
| `LaunchPageRetired` to `Ready` | 1190 | 992 | 1.20 |

**696 of the 1190 violations after the removal are on pages that had already been faulted before
it**, and 669 of the 1476 repeat pairs straddle it.

The mechanism is that deleting a KVM memory slot from a live VM calls
`kvm_arch_flush_shadow_memslot`, which invalidates every shadow page in the VM rather than only
the slot's, so the guest must re-fault everything it still has live. SOMA deletes a slot there
because retiring the launch page is a zero-length `KVM_SET_USER_MEMORY_REGION`, and that is the
point at which repair is committed.

## The decisive experiment

Retirement is two acts: erasing the host copy of the material, which is what makes repair
irreversible, and removing the slot. `SOMA_KVM_DEFER_LAUNCH_PAGE_SLOT` keeps the erasure exactly
where it is and moves only the removal to teardown. Both arms are one binary, interleaved.

| Measure | removal at the repair commit | removal deferred |
| --- | ---: | ---: |
| EPT violations, `RunStart` to `Ready` | 3228 and 3216 | 2484 and 2483 |
| **Distinct guest pages faulted** | **1746 and 1744** | **1745 and 1745** |
| Violations per distinct page | 1.85 and 1.84 | 1.42 and 1.42 |
| Pages faulted twice | 652 and 643 | 131 and 130 |
| Read faults | 2052 and 2050 | 1390 and 1389 |
| Write faults | 1176 and 1166 | 1094 and 1094 |

The violations fall by 23 percent **while the distinct pages the guest touches do not move at
all**. That is the mechanism manipulated and nothing else, and it is also the direct refutation of
the sentence this record set out to test: the cost is not proportional to the pages the resume
touches, because here the pages are identical and the cost is not.

## What it is worth

Receipt milestones, no tracing, cohorts interleaved on one host that other agents were also using.

`node:22` at 1024 MiB, 25 sequential samples per cohort, three interleaved rounds:

| Round | `machine_launched` to `ready`, at commit | deferred | TTI, at commit | deferred |
|---|---:|---:|---:|---:|
| 1 | 30.10 ms | 27.09 ms | 58.71 ms | 54.55 ms |
| 2 | 30.04 ms | 27.07 ms | 57.43 ms | 55.07 ms |
| 3 | 30.03 ms | 27.27 ms | 58.43 ms | 55.49 ms |
| median | **30.04 ms** | **27.09 ms** | **58.43 ms** | **55.07 ms** |

That is 2.95 ms off the segment and 3.36 ms off time to first command. The end-to-end saving being
at least as large as the segment saving is the point: the cost is removed rather than moved past
`ready` into the command window.

`busybox:stable-musl` at 1024 MiB, the same protocol, as a second shape:

| Round | segment, at commit | deferred | TTI, at commit | deferred |
|---|---:|---:|---:|---:|
| 1 | 30.31 ms | 27.57 ms | 35.10 ms | 33.55 ms |
| 2 | 29.25 ms | 26.96 ms | 34.09 ms | 32.27 ms |
| 3 | 30.28 ms | 26.60 ms | 34.92 ms | 31.85 ms |
| median | **30.28 ms** | **26.96 ms** | **34.92 ms** | **32.27 ms** |

3.32 ms off the segment against `node:22`'s 2.95 ms, and 2.65 ms off time to first command, at a
tenth of the workload. That is what a fixed cost being removed looks like.

`node:22` at concurrency 100, 100 sandboxes per cohort, four interleaved rounds:

| Round | segment, at commit | deferred | TTI, at commit | deferred | host busy |
|---|---:|---:|---:|---:|---:|
| 1 | 122.82 ms | 38.64 ms | 279.00 ms | 116.14 ms | 76% and 58% |
| 2 | 35.20 ms | 30.18 ms | 109.27 ms | 104.19 ms | 53% and 53% |
| 3 | 33.55 ms | 31.64 ms | 112.53 ms | 103.67 ms | 47% and 55% |
| 4 | 38.86 ms | 34.48 ms | 110.23 ms | 99.62 ms | 57% and 50% |
| median of 2 to 4 | **35.20 ms** | **31.64 ms** | **110.23 ms** | **103.67 ms** | |

> **Correction, 2026-09-01: do not quote the concurrency-100 column.** The author reported, after
> this table was written, that another job ran four cohorts of one hundred concurrently with the
> detached `c100.sh` run behind these figures. The concurrency-100 numbers are therefore
> contaminated and were never re-run, because the session ended first. The sequential column was
> not affected. Re-run the concurrency-100 arm on a verified-idle host before quoting it, and note
> that the one-minute load average is useless as a gate here because a hundred-sandbox cohort
> raises it by itself - sample `/proc/stat` while the run is idle instead.

Round 1 is discarded and shown: it is the first cohort of the session, its arms ran at 76 and 58
percent host busy against 47 to 57 for every other cohort, and its `at commit` arm is two and a
half times every other reading of the same thing. It is retained here rather than deleted because
its direction agrees and its magnitude does not, which is exactly the shape a contaminated cohort
has. On the three retained rounds the ordering is the same in every round.

## There is no way to pre-populate the extended page table on this host

The one upstream facility that populates EPT entries without a guest exit is
`KVM_PRE_FAULT_MEMORY`. Probed on eval-1 itself, `KVM_CHECK_EXTENSION` returns 0 for capability
236 and this kernel's capability numbering stops at `KVM_CAP_VM_TYPES` at 235, so the facility
does not exist here and adopting it is a host kernel question rather than a code question. It is
also a vCPU ioctl taking `vcpu->mutex`, so on a one-vCPU machine it would serialise against
`KVM_RUN` rather than overlap it; what share of a violation's cost it would recover is arithmetic
and has not been measured.

## What this changes and what it does not

The committed change makes retirement two acts and leaves the default byte-for-byte what shipped.
The saving is behind a lever because taking it means discharging a named obligation later than it
is discharged today: [ADR 0020](../adr/0020-launch-page-and-application-wire-contracts.md) requires
the trusted guest agent to wipe and unmap the page before executing user work, which it does at
`LaunchPageConsumed` about 8 ms into the resume and which nothing here touches, and
[ADR 0024](../adr/0024-per-instance-guest-responder-authority.md) lists memory-slot retirement
among the obligations protecting the responder secret. Under the lever the host copy is still
erased at the same instant and the slot still goes, at teardown instead of at the repair commit;
in between, guest physical `0xd0100000` is a mapped, zeroed, host-anonymous page. Whether that
discharges the obligation is an ADR decision and is deliberately not taken here.

## Retained artifacts

All under [`raw/2026-09-01-launch-page-slot-removal-ept-zap/`](raw/2026-09-01-launch-page-slot-removal-ept-zap/):

- `raw/ept-attribution.txt`, every attribution table above, both arms
- `raw/zap-alignment.txt`, the zap tracepoint and the per-millisecond violation rate against the
  machine's own milestones
- `raw/kvm-caps.txt`, the `KVM_CHECK_EXTENSION` probe, and `cap.c` beside it
- `cohorts/*.json`, every cohort's medians, per-sandbox TTI samples, and host busy fraction
- `pf.sh`, `cohort.sh`, `cohort.py`, `attrib.py`, `sym.py`, `zapalign.py`, `c100.sh`, `seg.py`,
  retained so every boundary these numbers measure is inspectable

## What this record does not prove

- It is one host, one commit, and two Generations. Nothing here is a claim about any other host.
- It is not a time-to-first-command result and must not be compared with a competitor figure.
- The `SOMA_KVM_TIMELINE` output the windows rest on is a diagnostic with no signature, no
  identity binding, and no stable schema.
- It does not show that deferring the removal is permitted. It shows what deferring it is worth.
- It says nothing about why the 1743 pages are 1743 rather than fewer. That question is open and
  the flat tail of 1705 faulting instruction pointers is the reason to expect it to be hard.
