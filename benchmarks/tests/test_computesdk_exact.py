import base64
import threading
import unittest

from benchmarks.computesdk_exact.slot import execute_slot
from benchmarks.computesdk_exact.statistics import computesdk_statistics, qualifies
from benchmarks.computesdk_exact.combine import combine
from benchmarks.computesdk_exact.run import BOUNDARY


class FakeClient:
    def __init__(self, answers):
        self.answers = iter(answers)
        self.calls = []

    def request(self, method, path, body=None):
        self.calls.append((method, path, body))
        return next(self.answers)


class ComputeSdkExactTests(unittest.TestCase):
    def test_statistics_match_the_computesdk_five_percent_trim(self) -> None:
        summary = computesdk_statistics(list(range(1, 101)))
        self.assertEqual(summary["trimmed_count"], 90)
        self.assertEqual(summary["median_ms"], 50.5)
        self.assertEqual(summary["p95_ms"], 91)
        self.assertEqual(summary["p99_ms"], 95)
        self.assertEqual(summary["raw_minimum_ms"], 1)
        self.assertEqual(summary["raw_maximum_ms"], 100)

    def test_slot_times_create_through_node_and_cleans_afterward(self) -> None:
        instance = "1" * 32
        cleanup = {
            "method": "forced", "machine": "complete", "memory": "complete",
            "storage": "complete", "network": {
                "lease": "not_owned", "runtime_attachment": "not_owned",
                "address_leases": "not_owned", "egress_policy": "not_owned",
                "dns_policy": "not_owned", "proxy_policy": "not_owned",
                "ingress_bindings": "not_owned",
            }, "guest_authority": "complete",
        }
        client = FakeClient([
            (201, {"schema": "soma.api.v1", "status": "ok", "result": {"instance_id": instance, "state": "ready"}, "receipt": {"preparation": {"value": "on_demand"}, "milestones": [{"kind": "ready", "elapsed_ns": 300}]}}),
            (200, {"schema": "soma.api.v1", "status": "ok", "result": {"instance_id": instance, "execution": {"exited": {"code": 0}}, "stdout": {"encoding": "base64", "byte_length": 9, "data": base64.b64encode(b"v22.0.0\n").decode()}, "stderr": {"encoding": "base64", "byte_length": 0, "data": ""}}, "receipt": {"milestones": [{"kind": "command_finished", "elapsed_ns": 900}]}}),
            (200, {"schema": "soma.api.v1", "status": "ok", "result": {"instance_id": instance, "state": "destroyed"}, "receipt": {"cleanup": cleanup}}),
        ])
        ticks = iter((1_000, 1_400, 2_000, 9_000))
        sample = execute_slot(client, threading.Barrier(1), clock=lambda: next(ticks))
        self.assertTrue(sample["successful"])
        self.assertTrue(sample["cleanup_complete"])
        self.assertEqual(sample["create_ns"], 400)
        self.assertEqual(sample["tti_ns"], 1_000)
        self.assertEqual(sample["cleanup_finished_ns"], 9_000)
        self.assertEqual(sample["launch_milestones"], [{"kind": "ready", "elapsed_ns": 300}])
        self.assertEqual(sample["command_milestones"], [{"kind": "command_finished", "elapsed_ns": 900}])
        self.assertEqual([call[0] for call in client.calls], ["POST", "POST", "DELETE"])

    def test_combining_shards_recomputes_one_trimmed_cohort(self) -> None:
        documents = [
            {
                "schema": "soma.computesdk-burst.v1",
                "boundary": BOUNDARY,
                "release_at_epoch_ns": 123,
                "samples": [
                    {
                        "instance_id": f"{value:032x}",
                        "command_succeeded": True,
                        "cleanup_complete": True,
                        "tti_ns": value * 1_000_000,
                    }
                    for value in values
                ],
            }
            for values in (range(1, 51), range(51, 101))
        ]

        result = combine(documents)

        self.assertEqual(result["attempted"], 100)
        self.assertEqual(result["cleanup_complete"], 100)
        self.assertEqual(result["statistics"]["p99_ms"], 95)

    def test_cleanup_failure_refuses_an_otherwise_successful_cohort(self) -> None:
        self.assertFalse(
            qualifies({"attempted": 100, "succeeded": 100, "cleanup_complete": 99})
        )

    def test_combining_anything_other_than_one_hundred_samples_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "100 samples"):
            combine(
                [
                    {
                        "schema": "soma.computesdk-burst.v1",
                        "boundary": BOUNDARY,
                        "release_at_epoch_ns": 123,
                        "samples": [],
                    }
                ]
            )

    def test_combining_missing_epoch_or_wrong_boundary_is_refused(self) -> None:
        samples = [
            {
                "instance_id": f"{value:032x}",
                "command_succeeded": True,
                "cleanup_complete": True,
                "tti_ns": value,
            }
            for value in range(100)
        ]
        with self.assertRaisesRegex(ValueError, "release epoch"):
            combine(
                [
                    {
                        "schema": "soma.computesdk-burst.v1",
                        "boundary": BOUNDARY,
                        "samples": samples,
                    }
                ]
            )

    def test_combining_missing_or_duplicate_instance_identity_is_refused(self) -> None:
        samples = [
            {
                "instance_id": f"{value:032x}",
                "command_succeeded": True,
                "cleanup_complete": True,
                "tti_ns": value,
            }
            for value in range(100)
        ]
        for invalid in (None, "not-an-instance", samples[1]["instance_id"]):
            changed = [dict(sample) for sample in samples]
            changed[0]["instance_id"] = invalid
            with self.assertRaisesRegex(ValueError, "canonical Instance|appears more than once"):
                combine(
                    [
                        {
                            "schema": "soma.computesdk-burst.v1",
                            "boundary": BOUNDARY,
                            "release_at_epoch_ns": 123,
                            "samples": changed,
                        }
                    ]
                )
        with self.assertRaisesRegex(ValueError, "timing boundary"):
            combine(
                [
                    {
                        "schema": "soma.computesdk-burst.v1",
                        "boundary": "almost exact",
                        "release_at_epoch_ns": 123,
                        "samples": samples,
                    }
                ]
            )


if __name__ == "__main__":
    unittest.main()
