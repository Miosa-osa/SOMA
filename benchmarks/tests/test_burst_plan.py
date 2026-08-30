import unittest

from benchmarks.local_alpha.burst.plan import BurstPlan

from .burst_fixtures import plan


class BurstPlanTests(unittest.TestCase):
    def test_declared_class_fixes_the_cache_state_and_burst_count(self) -> None:
        declared = plan(iterations=100, concurrency=10)

        self.assertEqual(declared.cache_state, "warm_host_page_cache")
        self.assertEqual(declared.bursts, 10)
        self.assertEqual(declared.as_dict()["preparation_class"], "warm-cache-restore")

    def test_a_warm_class_without_declared_preparation_fails_closed(self) -> None:
        for experiment_class in (
            "cold-cache-restore",
            "warm-cache-restore",
            "prepared-worker",
            "paused-pool",
            "ready-pool",
        ):
            with self.subTest(experiment_class=experiment_class):
                with self.assertRaisesRegex(ValueError, "prepared before the timer"):
                    plan(
                        experiment_class=experiment_class, prepared_before_timer=()
                    )

    def test_a_cold_build_class_that_declares_preparation_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "no preparation"):
            plan(
                experiment_class="cold-generation-build",
                prepared_before_timer=("a warm image",),
            )

    def test_a_cold_build_class_without_preparation_is_accepted(self) -> None:
        declared = plan(
            experiment_class="cold-generation-build", prepared_before_timer=()
        )

        self.assertEqual(declared.cache_state, "uncached_registry")
        self.assertEqual(declared.prepared_before_timer, ())

    def test_unknown_class_and_backend_fail_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "experiment class must be"):
            plan(experiment_class="steady-state")
        with self.assertRaisesRegex(ValueError, "backend must be"):
            plan(backend="firecracker")

    def test_iterations_must_be_a_positive_multiple_of_concurrency(self) -> None:
        for iterations, concurrency in ((100, 3), (5, 10), (0, 1)):
            with self.subTest(iterations=iterations, concurrency=concurrency):
                with self.assertRaises(ValueError):
                    plan(iterations=iterations, concurrency=concurrency)

    def test_a_relative_workload_executable_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "absolute guest executable"):
            plan(command=("busybox", "true"))
        with self.assertRaisesRegex(ValueError, "absolute guest executable"):
            plan(command=())

    def test_excluded_work_always_names_destruction_and_preparation(self) -> None:
        excluded = plan().excluded_work

        self.assertTrue(any("destruction" in item for item in excluded))
        self.assertIn(
            "preparation performed before the timer: the image was pulled "
            "before the timer",
            excluded,
        )

    def test_a_retained_plan_round_trips_through_its_record(self) -> None:
        declared = plan()

        self.assertEqual(BurstPlan.from_dict(declared.as_dict()), declared)


if __name__ == "__main__":
    unittest.main()
