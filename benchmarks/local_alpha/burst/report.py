"""The evidence document one or more validated burst cohorts can support."""

from __future__ import annotations

from collections.abc import Sequence
from pathlib import Path

from .document import (
    cohort_id,
    compact,
    cohort_rows,
    distinct_observations,
    failure_lines,
    generations,
    output_lines,
    raw_data_lines,
    stage_rows,
    table,
    timing_rows,
)
from .invocation import OPERATIONS, slot_calls
from .plan import BurstPlan
from .results import BurstResults, load_results, require_mergeable
from .slot import BOUNDARY


_COHORT_HEADER = (
    "Cohort",
    "Image",
    "Command",
    "Concurrency",
    "Iterations",
    "Successful",
    "Wall time (ns)",
)
_TIMING_HEADER = ("Cohort", "Success rate", "min", "p50", "p95", "p99", "max")
_STAGE_HEADER = ("Stage", "Samples", "min", "p50", "p95", "p99", "max")


def generate(paths: Sequence[Path], *, title: str) -> str:
    """Build one evidence document, refusing incomplete or class-mixed cohorts."""

    if not title.strip():
        raise ValueError("the document requires a title")
    cohorts = [load_results(path) for path in paths]
    require_mergeable(cohorts)
    for cohort in cohorts:
        BurstPlan.from_dict(cohort.plan)
    lines = [f"# {title} - {str(cohorts[0].metadata['started_at_utc'])[:10]}", ""]
    lines += _boundary(cohorts)
    lines += _identities(cohorts)
    lines += _invocation(cohorts[0])
    lines += _measured_boundary()
    lines += ["## Cohorts", ""]
    lines += table(_COHORT_HEADER, cohort_rows(cohorts), numeric_from=3) + [""]
    lines += ["## Time to first command", ""]
    lines += table(_TIMING_HEADER, timing_rows(cohorts)) + [""]
    lines += _stage_sections(cohorts)
    lines += ["## Command output", ""] + (
        output_lines(cohorts) or ["No cohort produced a successful command."]
    )
    lines += [""]
    lines += ["## Failures", ""] + failure_lines(cohorts) + [""]
    lines += ["## Raw data", ""] + raw_data_lines(cohorts) + [""]
    lines += _unproven(cohorts)
    return "\n".join(lines).rstrip() + "\n"


def _boundary(cohorts: Sequence[BurstResults]) -> list[str]:
    plan = cohorts[0].plan
    isolation = _isolation(cohorts)
    total = sum(len(cohort.samples) for cohort in cohorts)
    return [
        "## Evidence boundary",
        "",
        f"This result proves that the burst harness in `benchmarks/local_alpha/burst` "
        f"can open every slot of a burst at once against the `{plan['backend']}` "
        f"Backend through the `soma` command line, time each slot from immediately "
        f"before the create call to immediately after the workload command succeeded "
        f"inside the sandbox, destroy and verify every sandbox outside that timer, "
        f"retain all {total} attempted samples across "
        f"{_plural(len(cohorts), 'cohort')} with their failures, and refuse to "
        f"publish an incomplete or class-mixed run.",
        f"Every sample was declared as the `{plan['experiment_class']}` experiment "
        f"class with the `{plan['cache_state']}` cache state and observed "
        f"`{isolation}` isolation.",
        "",
        _label(plan, isolation),
        "It does not prove any latency objective.",
        "Read the final section before quoting any number here.",
        "",
    ]


def _label(plan: object, isolation: str) -> str:
    backend = plan["backend"]
    if backend == "kvm" and isolation == "hardware_virtual_machine":
        return (
            "These samples were taken on the SOMA KVM Backend with "
            "hardware virtual machine isolation."
        )
    return (
        f"These samples are a harness proof on the `{backend}` Backend with "
        f"`{isolation}` isolation. "
        "They are not a SOMA KVM performance result and they are not comparable "
        "to any provider benchmark, including the ComputeSDK Burst TTI benchmark."
    )


def _identities(cohorts: Sequence[BurstResults]) -> list[str]:
    metadata = cohorts[0].metadata
    soma = metadata["soma"]
    host = metadata["host"]
    binaries = soma["build_manifest"]["binaries"]
    cpu, memory, storage, kvm = (
        host["cpu"],
        host["memory"],
        host["storage"],
        host["kvm"],
    )
    plan = cohorts[0].plan
    return [
        "## Identities",
        "",
        f"- SOMA Git revision: `{soma['git_revision']}`, worktree clean: "
        f"`{soma['worktree_clean']}`.",
        f"- Measured binary: `{binaries['soma']['filename']}`, "
        f"{binaries['soma']['size_bytes']:,} bytes, SHA-256 "
        f"`{binaries['soma']['sha256']}`, built by "
        f"`{' '.join(soma['build_manifest']['build_argv'])}`.",
        f"- Cargo source SHA-256 `{soma['build_manifest']['source_sha256']}`; "
        f"benchmark harness SHA-256 "
        f"`{soma['build_manifest']['benchmark_sha256']}`.",
        f"- Host kernel: `{host['kernel']['sysname']} "
        f"{host['kernel']['release']} {host['kernel']['version']}` "
        f"{host['kernel']['machine']}.",
        f"- CPU: {cpu['model']}, {cpu['logical_cpus']} logical CPUs, microcode "
        f"`{cpu['microcode']}`.",
        f"- Memory: {memory['total']} total, {memory['available_at_start']} "
        f"available when the run started.",
        f"- Storage: mount `{storage.get('mount_point')}`, filesystem "
        f"`{storage.get('filesystem')}`, source `{storage.get('source')}`, "
        f"device `{storage.get('device_number')}` "
        f"({storage.get('device_model')}), super options "
        f"`{storage.get('super_options')}`.",
        f"- KVM: `/dev/kvm` present `{kvm['device_present']}`, readable by the "
        f"harness `{kvm['device_readable']}`, modules "
        f"`{', '.join(kvm['modules']) or 'none'}`.",
        f"- Backend probe: `{compact(metadata['backend_probe'])}`.",
        f"- Observed backend: `{compact(distinct_observations(cohorts, 'backend'))}`, "
        f"isolation `{compact(distinct_observations(cohorts, 'isolation'))}`, "
        f"preparation `{compact(distinct_observations(cohorts, 'preparation'))}`.",
        f"- Workload identity reported by the Backend: "
        f"`{compact(generations(cohorts))}`.",
        f"- Declared experiment class `{plan['experiment_class']}`, cache state "
        f"`{plan['cache_state']}`, network policy `{plan['network_policy']}`, "
        f"shape `{compact(plan['shape'])}`.",
        "- Prepared before the timer:",
        *(f"  - {item}." for item in plan["prepared_before_timer"]),
        "- Excluded from the timer:",
        *(
            f"  - {item}."
            for item in plan["excluded_work"]
            if not str(item).startswith("preparation performed before the timer:")
        ),
        "  - every preparation listed above.",
        "",
    ]


def _invocation(cohort: BurstResults) -> list[str]:
    plan = BurstPlan.from_dict(cohort.plan)
    calls = slot_calls(
        plan,
        soma_binary=Path("$SOMA_BIN"),
        state_root=Path("$STATE_ROOT"),
        instance_id="$INSTANCE_ID",
        operation_ids={operation: "$OPERATION_ID" for operation in OPERATIONS},
    )
    return [
        "## Invocation",
        "",
        "Each slot runs exactly these three processes, with fresh identities:",
        "",
        "```sh",
        *(" ".join(calls[operation]) for operation in OPERATIONS),
        "```",
        "",
    ]


def _measured_boundary() -> list[str]:
    return [
        "## Measured boundary",
        "",
        f"The time-to-first-command clock is `time.perf_counter_ns` in the slot's "
        f"own thread with the boundary `{BOUNDARY}`.",
        "A cohort of N iterations at concurrency C runs N divided by C bursts, "
        "and every slot of a burst is released by one barrier and creates its own "
        "sandbox; the cohort table names N and C for each cohort.",
        "Wall time covers the whole cohort including the excluded destruction.",
        "Every percentile is nearest rank over successful samples only, so p99 of "
        "100 samples is the 99th ordered value and p99 of 10 samples is the "
        "largest.",
        "Stage rows are the facade milestones the receipts carry; the harness "
        "overhead row is the measured time to first command minus the launch and "
        "exec facade totals, which is the cost of two process spawns and their "
        "response reading.",
        "",
    ]


def _stage_sections(cohorts: Sequence[BurstResults]) -> list[str]:
    lines = ["## Stage timings (ns)", ""]
    for cohort in cohorts:
        lines += [f"### {cohort_id(cohort)}", ""]
        lines += table(_STAGE_HEADER, stage_rows(cohort)) + [""]
    return lines


def _plural(count: int, noun: str) -> str:
    return f"{count} {noun}" if count == 1 else f"{count} {noun}s"


def _unproven(cohorts: Sequence[BurstResults]) -> list[str]:
    plan = cohorts[0].plan
    isolation = _isolation(cohorts)
    lines = ["## What this does not prove", ""]
    if plan["backend"] != "kvm" or isolation != "hardware_virtual_machine":
        lines.append(
            f"- This is a harness proof on the `{plan['backend']}` Backend with "
            f"`{isolation}` isolation. "
            "It is not a SOMA KVM performance result, it is not a virtual machine "
            "measurement, and it is not comparable to any provider benchmark, "
            "including the ComputeSDK Burst TTI benchmark."
        )
    lines += [
        "- The timer includes two `soma` process spawns and their response reading "
        "per sample; a provider SDK measurement does not pay that cost, and the "
        "harness overhead stage row states how large it is here.",
        "- Destruction is excluded from every time-to-first-command value. "
        "It was executed and its cleanup evidence verified for every sample, and "
        "it is inside the reported wall time.",
        "- Percentiles are nearest rank over successful samples only. "
        "Failures are listed above and are never merged into the distribution.",
        f"- Each cohort was produced once, on one host, on "
        f"{str(cohorts[0].metadata['started_at_utc'])[:10]}, without quiescing the "
        "host; no repetition on a second host or a second day exists.",
    ]
    if not any(
        identity.get("generation_id") for identity in generations(cohorts)
    ):
        lines.append(
            "- The Backend reported no Generation identity, so no Generation digest "
            "is bound to these samples."
        )
    lines.append("")
    return lines


def _isolation(cohorts: Sequence[BurstResults]) -> str:
    observed = distinct_observations(cohorts, "isolation")
    values = [
        item["value"]
        for item in observed
        if isinstance(item, dict) and item.get("state") == "observed"
    ]
    return "/".join(str(value) for value in values) or "unobserved"
