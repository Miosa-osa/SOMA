import unittest

from benchmarks.local_alpha.statistics import (
    BenchmarkStatistics,
    nearest_rank,
    summarize,
)


class BenchmarkStatisticsTests(unittest.TestCase):
    def test_summary_counts_failures_and_computes_integer_nanosecond_statistics(self) -> None:
        summary = summarize((50, 10, 40, 20, 30), failed_count=2)

        self.assertEqual(
            summary,
            BenchmarkStatistics(
                accepted_count=5,
                failed_count=2,
                total_count=7,
                success_rate=5 / 7,
                minimum_ns=10,
                maximum_ns=50,
                median_ns=30,
                p95_ns=50,
                p99_ns=50,
            ),
        )
        self.assertEqual(
            summary.as_dict(),
            {
                "accepted_count": 5,
                "failed_count": 2,
                "total_count": 7,
                "success_rate": 5 / 7,
                "minimum_ns": 10,
                "maximum_ns": 50,
                "median_ns": 30,
                "p95_ns": 50,
                "p99_ns": 50,
            },
        )

    def test_even_sample_count_uses_the_arithmetic_median(self) -> None:
        summary = summarize((4, 1, 2, 3))

        self.assertEqual(summary.median_ns, 2.5)
        self.assertEqual(summary.success_rate, 1.0)

    def test_nearest_rank_percentiles_are_deterministic_at_exact_boundaries(self) -> None:
        summary = summarize(range(1, 101))

        self.assertEqual(summary.p95_ns, 95)
        self.assertEqual(summary.p99_ns, 99)

    def test_nearest_rank_percentiles_select_the_maximum_for_two_samples(self) -> None:
        summary = summarize((100, 1))

        self.assertEqual(summary.p95_ns, 100)
        self.assertEqual(summary.p99_ns, 100)

    def test_zero_duration_single_sample_is_valid(self) -> None:
        summary = summarize((0,))

        self.assertEqual(summary.minimum_ns, 0)
        self.assertEqual(summary.maximum_ns, 0)
        self.assertEqual(summary.median_ns, 0)
        self.assertEqual(summary.p95_ns, 0)
        self.assertEqual(summary.p99_ns, 0)

    def test_empty_sample_input_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least one"):
            summarize((), failed_count=3)

    def test_non_integer_boolean_and_negative_samples_fail_closed(self) -> None:
        for samples in ((1, -1), (1, 1.5), (1, True), (1, "2")):
            with self.subTest(samples=samples):
                with self.assertRaises(ValueError):
                    summarize(samples)  # type: ignore[arg-type]

    def test_nearest_rank_returns_the_ceiling_ranked_value(self) -> None:
        self.assertEqual(nearest_rank([1, 2, 3, 4], 50), 2)
        self.assertEqual(nearest_rank([1, 2, 3, 4], 100), 4)
        self.assertEqual(nearest_rank([5], 1), 5)

    def test_nearest_rank_rejects_an_empty_cohort_or_a_bad_percentile(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least one"):
            nearest_rank([], 50)
        for percentile in (0, 101, 1.5, True):
            with self.subTest(percentile=percentile):
                with self.assertRaises(ValueError):
                    nearest_rank([1], percentile)  # type: ignore[arg-type]

    def test_invalid_failed_count_fails_closed(self) -> None:
        for failed_count in (-1, 1.5, True):
            with self.subTest(failed_count=failed_count):
                with self.assertRaises(ValueError):
                    summarize((1,), failed_count=failed_count)  # type: ignore[arg-type]


if __name__ == "__main__":
    unittest.main()
