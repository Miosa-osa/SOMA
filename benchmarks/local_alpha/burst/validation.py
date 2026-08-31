"""Fail-closed validation for retained burst records."""

from __future__ import annotations

from collections.abc import Mapping, Sequence

from .plan import EXPERIMENT_CLASSES


METADATA_FIELDS = (
    "run_id",
    "started_at_utc",
    "plan",
    "soma",
    "engine",
    "host",
    "backend_probe",
)
PLAN_FIELDS = (
    "experiment_class",
    "preparation_class",
    "prepared_before_timer",
    "cache_state",
    "backend",
    "image",
    "command",
    "network_policy",
    "shape",
    "iterations",
    "concurrency",
    "bursts",
    "timeout_ms",
    "max_output_bytes",
    "excluded_work",
)
SOMA_FIELDS = ("git_revision", "worktree_clean", "build_manifest")
ENGINE_FIELDS = (
    "schema",
    "generation_store",
    "head_directory",
    "allow_uncertified_generation",
)
HOST_FIELDS = ("kernel", "cpu", "memory", "storage", "kvm")


def validate_metadata(
    metadata: Mapping[str, object], *, require_engine: bool
) -> None:
    """Validate one metadata record for its declared schema generation."""

    required_fields = METADATA_FIELDS
    if not require_engine:
        required_fields = tuple(field for field in METADATA_FIELDS if field != "engine")
    require(metadata, required_fields, "run metadata")
    plan = metadata["plan"]
    soma = metadata["soma"]
    engine = metadata.get("engine")
    host = metadata["host"]
    require(plan, PLAN_FIELDS, "run metadata plan")
    require(soma, SOMA_FIELDS, "run metadata soma identity")
    if require_engine:
        require(engine, ENGINE_FIELDS, "run metadata engine identity")
        validate_engine(engine)
    elif engine is not None:
        validate_engine(engine)
    require(host, HOST_FIELDS, "run metadata host identity")
    if plan["experiment_class"] not in EXPERIMENT_CLASSES:
        raise ValueError("run metadata declares an unknown experiment class")
    if plan["preparation_class"] != plan["experiment_class"]:
        raise ValueError("run metadata preparation class contradicts its experiment class")
    prepared = plan["prepared_before_timer"]
    if not isinstance(prepared, list):
        raise ValueError("run metadata preparation must be a list")
    if plan["experiment_class"] != "cold-generation-build" and not prepared:
        raise ValueError(
            f"class {plan['experiment_class']} must record what was prepared "
            "before the timer"
        )
    if not plan["excluded_work"]:
        raise ValueError("run metadata must name the work excluded from the timer")


def validate_engine(engine: object) -> None:
    """Validate non-secret identities for effective runtime engine settings."""

    if not isinstance(engine, Mapping) or engine.get("schema") != "soma.engine-settings.v1":
        raise ValueError("run metadata engine identity has an unknown schema")
    if type(engine.get("allow_uncertified_generation")) is not bool:
        raise ValueError("run metadata engine opt-in must be boolean")
    for name in ("generation_store", "head_directory"):
        locator = engine.get(name)
        if not isinstance(locator, Mapping):
            raise ValueError(f"run metadata engine {name} must be an object")
        state = locator.get("state")
        if state == "unset" and set(locator) == {"state"}:
            continue
        digest = locator.get("locator_sha256")
        if (
            state != "configured"
            or set(locator) != {"state", "locator_sha256"}
            or not isinstance(digest, str)
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
        ):
            raise ValueError(f"run metadata engine {name} identity is invalid")


def validate_samples(
    metadata: Mapping[str, object],
    samples: Sequence[Mapping[str, object]],
    completion: Mapping[str, object],
    *,
    require_attribution: bool = False,
) -> None:
    """Validate completeness, identity, outcomes, and cleanup for all samples."""

    plan = metadata["plan"]
    iterations = plan["iterations"]
    if completion.get("attempted") != len(samples) or len(samples) != iterations:
        raise ValueError(
            f"run is incomplete: {len(samples)} of {iterations} samples were retained"
        )
    repetitions = sorted(int(sample["repetition"]) for sample in samples)
    if repetitions != list(range(1, iterations + 1)):
        raise ValueError("sample repetitions must cover the cohort exactly once")
    for sample in samples:
        if sample.get("experiment_class") != plan["experiment_class"]:
            raise ValueError("results merge different experiment classes")
        if sample["successful"]:
            _require_command(sample)
        elif not sample.get("failures"):
            raise ValueError("an unsuccessful sample lacks a typed failure reason")
        elif require_attribution:
            _require_attribution(sample)
    if require_attribution and completion.get("failure_breakdown") is None:
        raise ValueError("a completed run must carry its failure breakdown")


def require(value: object, fields: Sequence[str], label: str) -> None:
    """Require a mapping with every named field populated."""

    if not isinstance(value, Mapping):
        raise ValueError(f"{label} must be an object")
    for field in fields:
        if value.get(field) in (None, "", {}):
            raise ValueError(f"{label} is missing required field: {field}")


def _require_attribution(sample: Mapping[str, object]) -> None:
    """Refuse a failure that names no cause.

    A typed reason says which step refused. It does not say why, and a run whose every sample
    carries `launch_process_failed` and nothing else is a zero no one can act on.
    """

    for failure in sample["failures"]:
        if not str(failure.get("detail") or "").strip():
            raise ValueError(
                f"failure {failure.get('reason')} at {failure.get('operation')} "
                "carries no attributable detail"
            )


def _require_command(sample: Mapping[str, object]) -> None:
    command = sample.get("command")
    if (
        not isinstance(command, Mapping)
        or command.get("status") != "exited"
        or command.get("exit_code") != 0
        or not isinstance(command.get("stdout"), Mapping)
        or type(sample.get("tti_ns")) is not int
        or sample["tti_ns"] < 0
        or not sample.get("cleanup_complete")
    ):
        raise ValueError("a successful sample lacks workload command evidence")
