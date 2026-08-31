import tempfile
import threading
import unittest
from pathlib import Path

from benchmarks.local_alpha.burst.results import load_results
from benchmarks.local_alpha.burst.run import run_burst
from benchmarks.local_alpha.burst.slot import BurstSample

from benchmarks.tests.burst_fixtures import metadata_record, plan


def _sample(instance_id: str, *, successful: bool, tti_ns: int) -> BurstSample:
    return BurstSample(
        instance_id=instance_id,
        operation_ids={},
        tti_ns=tti_ns,
        command_succeeded=successful,
        cleanup_complete=True,
        processes={},
        stages={},
        observed={"backend": "docker_container"},
        command={
            "status": "exited" if successful else "timed_out",
            "exit_code": 0 if successful else None,
            "stdout": {"encoding": "base64", "byte_length": 0, "data_base64": ""},
            "stderr": {"encoding": "base64", "byte_length": 0, "data_base64": ""},
        },
        failures=()
        if successful
        else (
            {
                "reason": "command_unsuccessful",
                "operation": "exec",
                "detail": "timed_out:None",
            },
        ),
    )


class BurstRunTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = Path(
            self.enterContext(tempfile.TemporaryDirectory(prefix="soma-burst-run-"))
        )

    def _run(self, declared, slot):
        return run_burst(
            declared,
            soma_binary=Path("/bin/soma"),
            state_root=self.directory,
            environment={},
            metadata=metadata_record(declared),
            results_path=self.directory / "results.jsonl",
            slot=slot,
        )

    def test_every_slot_of_a_burst_runs_concurrently(self) -> None:
        declared = plan(iterations=6, concurrency=3)
        rendezvous = threading.Barrier(declared.concurrency, timeout=30)
        instances: list[str] = []
        lock = threading.Lock()

        def slot(_declared, *, instance_id, **_kwargs):
            rendezvous.wait()
            with lock:
                instances.append(instance_id)
            return _sample(instance_id, successful=True, tti_ns=len(instances))

        summary = self._run(declared, slot)

        self.assertEqual(summary["attempted"], 6)
        self.assertEqual(len(set(instances)), 6)
        self.assertFalse(rendezvous.broken)

    def test_failed_samples_are_written_and_counted(self) -> None:
        declared = plan(iterations=4, concurrency=2)
        order = iter(range(1, 5))

        def slot(_declared, *, instance_id, **_kwargs):
            index = next(order)
            return _sample(instance_id, successful=index % 2 == 1, tti_ns=index)

        summary = self._run(declared, slot)
        results = load_results(self.directory / "results.jsonl")

        self.assertEqual(summary["attempted"], 4)
        self.assertEqual(summary["tti"]["accepted_count"], 2)
        self.assertEqual(summary["tti"]["failed_count"], 2)
        self.assertEqual(len(results.samples), 4)
        self.assertEqual(len(results.failed), 2)
        self.assertEqual(
            sorted(sample["burst_index"] for sample in results.samples), [0, 0, 1, 1]
        )
        self.assertEqual(
            sorted(sample["slot_index"] for sample in results.samples), [0, 0, 1, 1]
        )

    def test_the_completion_record_reports_the_whole_wall_time(self) -> None:
        declared = plan(iterations=2, concurrency=1)

        def slot(_declared, *, instance_id, **_kwargs):
            return _sample(instance_id, successful=True, tti_ns=5)

        summary = self._run(declared, slot)

        self.assertGreater(summary["wall_ns"], 0)
        self.assertEqual(summary["cleanup_complete_count"], 2)
        self.assertEqual(summary["command_succeeded_count"], 2)
        self.assertEqual(summary["tti"]["percentile_method"], "nearest_rank")


if __name__ == "__main__":
    unittest.main()
