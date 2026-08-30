import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.burst.results import (
    ResultsWriter,
    load_results,
    require_mergeable,
    statistics,
)

from .burst_fixtures import (
    complete_results,
    completion_record,
    metadata_record,
    plan,
    sample_record,
    write_results,
)


class BurstStatisticsTests(unittest.TestCase):
    def test_nearest_rank_percentiles_match_a_known_array(self) -> None:
        summary = statistics(list(range(1, 101)), failed_count=0)

        self.assertEqual(summary["minimum_ns"], 1)
        self.assertEqual(summary["p50_ns"], 50)
        self.assertEqual(summary["p95_ns"], 95)
        self.assertEqual(summary["p99_ns"], 99)
        self.assertEqual(summary["maximum_ns"], 100)
        self.assertEqual(summary["percentile_method"], "nearest_rank")

    def test_nearest_rank_selects_the_maximum_for_a_small_cohort(self) -> None:
        summary = statistics([9, 1, 5, 3], failed_count=1)

        self.assertEqual(summary["p50_ns"], 3)
        self.assertEqual(summary["p95_ns"], 9)
        self.assertEqual(summary["p99_ns"], 9)
        self.assertEqual(summary["total_count"], 5)
        self.assertEqual(summary["success_rate"], 0.8)

    def test_a_cohort_without_accepted_samples_reports_no_percentiles(self) -> None:
        summary = statistics([], failed_count=3)

        self.assertEqual(summary["success_rate"], 0.0)
        for name in ("minimum_ns", "p50_ns", "p95_ns", "p99_ns", "maximum_ns"):
            self.assertIsNone(summary[name])

    def test_a_negative_failure_count_fails_closed(self) -> None:
        with self.assertRaises(ValueError):
            statistics([1], failed_count=-1)


class BurstResultsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = Path(
            self.enterContext(tempfile.TemporaryDirectory(prefix="soma-burst-test-"))
        )

    def path(self, name: str = "results.jsonl") -> Path:
        return self.directory / name

    def test_written_results_round_trip_through_the_jsonl_file(self) -> None:
        declared = plan(iterations=3, concurrency=1)
        samples = [
            sample_record(declared, index, tti_ns=100 * index) for index in (1, 2, 3)
        ]
        path = write_results(self.path(), declared, samples)

        results = load_results(path)

        self.assertEqual(len(results.samples), 3)
        self.assertEqual(results.experiment_class, "warm-cache-restore")
        self.assertEqual(results.wall_ns, 9_000)
        self.assertEqual(
            [sample["tti_ns"] for sample in results.samples], [100, 200, 300]
        )
        self.assertEqual(results.metadata["plan"], declared.as_dict())
        self.assertEqual(results.tti_statistics()["p50_ns"], 200)

    def test_failed_samples_are_retained_and_never_summarized_away(self) -> None:
        declared = plan(iterations=4, concurrency=2)
        samples = [
            sample_record(declared, 1, tti_ns=10),
            sample_record(declared, 2, successful=False, tti_ns=20),
            sample_record(declared, 3, tti_ns=30),
            sample_record(declared, 4, successful=False, tti_ns=40),
        ]

        results = load_results(write_results(self.path(), declared, samples))
        summary = results.tti_statistics()

        self.assertEqual(len(results.samples), 4)
        self.assertEqual(len(results.failed), 2)
        self.assertEqual(summary["accepted_count"], 2)
        self.assertEqual(summary["failed_count"], 2)
        self.assertEqual(summary["total_count"], 4)
        self.assertEqual(summary["success_rate"], 0.5)
        self.assertEqual(
            results.failed[0]["failures"][0]["reason"], "command_unsuccessful"
        )

    def test_mixed_experiment_classes_in_one_file_fail_closed(self) -> None:
        declared = plan(iterations=2, concurrency=1)
        samples = [
            sample_record(declared, 1),
            sample_record(declared, 2, experiment_class="paused-pool"),
        ]

        with self.assertRaisesRegex(ValueError, "different experiment classes"):
            load_results(write_results(self.path(), declared, samples))

    def test_two_concatenated_runs_fail_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        first = write_results(self.path("a.jsonl"), declared, [sample_record(declared, 1)])
        second = write_results(
            self.path("b.jsonl"), declared, [sample_record(declared, 1)]
        )
        merged = self.path("merged.jsonl")
        merged.write_bytes(first.read_bytes() + second.read_bytes())

        with self.assertRaisesRegex(ValueError, "exactly one run metadata record"):
            load_results(merged)

    def test_a_successful_sample_without_a_command_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        for broken in (
            {"command": None},
            {"command": {"status": "timed_out", "exit_code": None, "stdout": {}}},
            {"tti_ns": None},
            {"cleanup_complete": False},
        ):
            with self.subTest(broken=sorted(broken)):
                path = self.path(f"{abs(hash(str(broken)))}.jsonl")
                write_results(
                    path, declared, [sample_record(declared, 1, **broken)]
                )
                with self.assertRaisesRegex(ValueError, "workload command evidence"):
                    load_results(path)

    def test_an_unsuccessful_sample_without_a_reason_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        path = write_results(
            self.path(),
            declared,
            [sample_record(declared, 1, successful=False, failures=[])],
        )

        with self.assertRaisesRegex(ValueError, "typed failure reason"):
            load_results(path)

    def test_incomplete_metadata_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        for field in ("host", "soma", "backend_probe", "started_at_utc"):
            with self.subTest(field=field):
                metadata = metadata_record(declared)
                metadata.pop(field)
                path = self.path(f"{field}.jsonl")
                write_results(
                    path,
                    declared,
                    [sample_record(declared, 1)],
                    metadata=metadata,
                )
                with self.assertRaisesRegex(ValueError, f"required field: {field}"):
                    load_results(path)

    def test_metadata_missing_a_host_identity_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        metadata = metadata_record(declared)
        metadata["host"].pop("storage")
        path = write_results(
            self.path(), declared, [sample_record(declared, 1)], metadata=metadata
        )

        with self.assertRaisesRegex(ValueError, "host identity is missing"):
            load_results(path)

    def test_a_run_without_its_completion_record_fails_closed(self) -> None:
        declared = plan(iterations=2, concurrency=1)
        path = self.path()
        with ResultsWriter(path) as writer:
            writer.append(metadata_record(declared))
            writer.append(sample_record(declared, 1))

        with self.assertRaisesRegex(ValueError, "run is incomplete"):
            load_results(path)

    def test_a_short_cohort_fails_closed(self) -> None:
        declared = plan(iterations=4, concurrency=2)
        path = write_results(
            self.path(),
            declared,
            [sample_record(declared, 1), sample_record(declared, 2)],
        )

        with self.assertRaisesRegex(ValueError, "2 of 4 samples"):
            load_results(path)

    def test_merging_cohorts_of_different_classes_fails_closed(self) -> None:
        warm = plan(iterations=1, concurrency=1)
        cold = plan(
            iterations=1, concurrency=1, experiment_class="paused-pool"
        )
        first = load_results(
            write_results(self.path("warm.jsonl"), warm, [sample_record(warm, 1)])
        )
        second = load_results(
            write_results(self.path("cold.jsonl"), cold, [sample_record(cold, 1)])
        )

        require_mergeable([first])
        with self.assertRaisesRegex(ValueError, "different experiment_class"):
            require_mergeable([first, second])

    def test_merging_cohorts_from_different_hosts_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        first = load_results(
            write_results(self.path("a.jsonl"), declared, [sample_record(declared, 1)])
        )
        elsewhere = metadata_record(declared)
        elsewhere["host"]["kernel"]["release"] = "6.8.0-generic"
        second = load_results(
            write_results(
                self.path("b.jsonl"),
                declared,
                [sample_record(declared, 1)],
                metadata=elsewhere,
            )
        )

        with self.assertRaisesRegex(ValueError, "different host identities"):
            require_mergeable([first, second])

    def test_merging_cohorts_taken_minutes_apart_is_allowed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        first = load_results(
            write_results(self.path("a.jsonl"), declared, [sample_record(declared, 1)])
        )
        later = metadata_record(declared)
        later["host"]["memory"]["available_at_start"] = "2 kB"
        second = load_results(
            write_results(
                self.path("b.jsonl"),
                declared,
                [sample_record(declared, 1)],
                metadata=later,
            )
        )

        require_mergeable([first, second])

    def test_an_empty_or_foreign_results_file_fails_closed(self) -> None:
        empty = self.path("empty.jsonl")
        empty.write_bytes(b"")
        foreign = self.path("foreign.jsonl")
        foreign.write_bytes(b'{"schema":"other.v1"}\n')

        with self.assertRaisesRegex(ValueError, "results file is empty"):
            load_results(empty)
        with self.assertRaisesRegex(ValueError, "unknown schema"):
            load_results(foreign)

    def test_the_writer_never_overwrites_an_existing_results_file(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        path = complete_results(self.path(), plan(iterations=1, concurrency=1))

        with self.assertRaises(FileExistsError):
            write_results(path, declared, [sample_record(declared, 1)])

    def test_completion_and_sample_counts_must_agree(self) -> None:
        declared = plan(iterations=2, concurrency=1)
        path = write_results(
            self.path(),
            declared,
            [sample_record(declared, 1), sample_record(declared, 2)],
            completion=completion_record(declared, 1),
        )

        with self.assertRaisesRegex(ValueError, "run is incomplete"):
            load_results(path)


if __name__ == "__main__":
    unittest.main()
