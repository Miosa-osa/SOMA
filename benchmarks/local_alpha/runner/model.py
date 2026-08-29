"""Runner results passed from samplers to artifact aggregation."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class SampleOutcome:
    instance_id: str
    operation_ids: dict[str, str]
    duration_ns: int
    boundary: str
    accepted: bool
    cleanup_validated: bool
    operations: tuple[dict[str, object], ...]
    receipt_metrics_ns: dict[str, int]
    errors: tuple[dict[str, str], ...]

    def as_record(
        self,
        *,
        run_id: str,
        sample_id: str,
        scenario_id: str,
        repetition: int,
    ) -> dict[str, object]:
        return {
            "schema": "soma.local-alpha.raw.v1",
            "record_type": "sample",
            "run_id": run_id,
            "sample_id": sample_id,
            "scenario_id": scenario_id,
            "repetition": repetition,
            "cache_state": "cached",
            "accepted": self.accepted,
            "duration_ns": self.duration_ns,
            "external_tti": {
                "clock": "time.perf_counter_ns",
                "boundary": self.boundary,
                "duration_ns": self.duration_ns,
            },
            "instance_id": self.instance_id,
            "operation_ids": self.operation_ids,
            "cleanup_validated": self.cleanup_validated,
            "receipt_metrics_ns": self.receipt_metrics_ns,
            "operations": list(self.operations),
            "errors": list(self.errors),
        }
