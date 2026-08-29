import gc
import unittest
import weakref

from benchmarks.local_alpha.runner.model import SampleOutcome
from benchmarks.local_alpha.runner.summary import CompactSample, metric_summaries


def _outcome(
    duration: int,
    *,
    accepted: bool = True,
    metric: int | None = None,
    preparation: str | None = None,
    payload: object | None = None,
) -> SampleOutcome:
    operation: dict[str, object] = {}
    if preparation is not None:
        operation["preparation_class"] = preparation
    if payload is not None:
        operation["process"] = payload
    return SampleOutcome(
        instance_id=f"{duration:032x}",
        operation_ids={"run": f"{duration + 100:032x}"},
        duration_ns=duration,
        boundary="test",
        accepted=accepted,
        cleanup_validated=accepted,
        operations=(operation,) if operation else (),
        receipt_metrics_ns={} if metric is None else {"run.command": metric},
        errors=() if accepted else ({"operation": "run", "type": "failure"},),
    )


class RunnerSummaryTests(unittest.TestCase):
    def test_statistics_include_failures_and_available_receipt_metrics(self) -> None:
        samples = tuple(
            CompactSample.from_outcome(item)
            for item in (
                _outcome(10, metric=3),
                _outcome(30),
                _outcome(20, accepted=False, metric=4),
            )
        )

        metrics = metric_summaries(samples)

        self.assertEqual(metrics["external_tti"]["median_ns"], 20)
        self.assertEqual(metrics["external_tti"]["p99_ns"], 30)
        self.assertEqual(metrics["external_tti"]["success_rate"], 2 / 3)
        self.assertEqual(metrics["receipt"]["run.command"]["accepted_count"], 1)
        self.assertEqual(metrics["receipt"]["run.command"]["failed_count"], 2)

    def test_compact_sample_does_not_retain_operation_capture_payloads(self) -> None:
        class Payload:
            pass

        payload = Payload()
        retained = weakref.ref(payload)
        outcome = _outcome(
            10,
            metric=3,
            preparation="on_demand",
            payload=payload,
        )

        compact = CompactSample.from_outcome(outcome)
        del outcome, payload
        gc.collect()

        self.assertIsNone(retained())
        self.assertFalse(hasattr(compact, "operations"))
        self.assertEqual(compact.preparation_classes, ("on_demand",))

    def test_summary_rejects_mixed_observed_preparation_classes(self) -> None:
        samples = (
            CompactSample.from_outcome(_outcome(10, preparation="on_demand")),
            CompactSample.from_outcome(_outcome(11, preparation="prepared_worker")),
        )

        with self.assertRaisesRegex(ValueError, "preparation"):
            metric_summaries(samples)


if __name__ == "__main__":
    unittest.main()
