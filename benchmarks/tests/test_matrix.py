import unittest

from benchmarks.local_alpha.matrix import (
    BASE_IMAGES,
    BASE_NETWORK_POLICIES,
    BASE_SHAPES,
    build_burst_cohorts,
    build_scenario_matrix,
)


class ScenarioMatrixTests(unittest.TestCase):
    def test_matrix_covers_every_required_base_combination(self) -> None:
        scenarios = build_scenario_matrix()
        base = [scenario for scenario in scenarios if scenario.kind == "base"]

        observed = {
            (
                scenario.caller,
                scenario.mode,
                scenario.image,
                scenario.shape.vcpus,
                scenario.shape.memory_mib,
                scenario.network_policy,
            )
            for scenario in base
        }
        expected = {
            (caller, mode, image, shape.vcpus, shape.memory_mib, network)
            for caller in ("cli", "mcp")
            for mode in ("one_shot", "managed")
            for image in BASE_IMAGES
            for shape in BASE_SHAPES
            for network in BASE_NETWORK_POLICIES
        }

        self.assertEqual(observed, expected)
        self.assertEqual(len(base), 48)

    def test_adverse_matrix_covers_binary_and_bounded_failures_for_both_callers(self) -> None:
        adverse = [
            scenario
            for scenario in build_scenario_matrix()
            if scenario.kind == "adverse"
        ]

        self.assertEqual(len(adverse), 8)
        self.assertEqual(
            {(scenario.caller, scenario.workload.name) for scenario in adverse},
            {
                (caller, workload)
                for caller in ("cli", "mcp")
                for workload in (
                    "binary_output",
                    "nonzero_exit",
                    "output_limit",
                    "timeout",
                )
            },
        )

    def test_scenario_ids_are_unique_and_path_safe(self) -> None:
        identifiers = [scenario.identifier for scenario in build_scenario_matrix()]

        self.assertEqual(len(identifiers), len(set(identifiers)))
        self.assertTrue(all(identifier.replace("-", "").isalnum() for identifier in identifiers))

    def test_bursts_are_bounded_and_cover_both_callers_modes_and_images(self) -> None:
        cohorts = build_burst_cohorts((4, 8))

        self.assertEqual({cohort.width for cohort in cohorts}, {4, 8})
        self.assertEqual(
            {(cohort.caller, cohort.mode, cohort.image) for cohort in cohorts},
            {
                (caller, mode, image)
                for caller in ("cli", "mcp")
                for mode in ("one_shot", "managed")
                for image in BASE_IMAGES
            },
        )

    def test_invalid_burst_width_fails_closed(self) -> None:
        with self.assertRaises(ValueError):
            build_burst_cohorts((0,))
        with self.assertRaises(ValueError):
            build_burst_cohorts((33,))


if __name__ == "__main__":
    unittest.main()
