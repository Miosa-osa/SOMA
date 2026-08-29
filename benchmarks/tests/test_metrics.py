import unittest

from benchmarks.local_alpha.metrics import (
    Milestone,
    duration_ns,
    parse_milestones,
    run_metrics,
)


def complete_run_receipt() -> dict[str, object]:
    return {
        "milestones": [
            {"kind": "accepted", "elapsed_ns": 0},
            {"kind": "workload_resolved", "elapsed_ns": 10},
            {"kind": "admitted", "elapsed_ns": 20},
            {"kind": "machine_launched", "elapsed_ns": 30},
            {"kind": "ready", "elapsed_ns": 40},
            {"kind": "command_started", "elapsed_ns": 50},
            {"kind": "command_finished", "elapsed_ns": 70},
            {"kind": "cleanup_started", "elapsed_ns": 75},
            {"kind": "cleanup_finished", "elapsed_ns": 90},
        ]
    }


class ReceiptMetricTests(unittest.TestCase):
    def test_parser_returns_validated_immutable_milestones(self) -> None:
        parsed = parse_milestones(complete_run_receipt())

        self.assertIsInstance(parsed, tuple)
        self.assertEqual(parsed[0], Milestone(kind="accepted", elapsed_ns=0))
        self.assertEqual(parsed[-1], Milestone(kind="cleanup_finished", elapsed_ns=90))

    def test_duration_and_complete_run_metrics_use_explicit_boundaries(self) -> None:
        receipt = complete_run_receipt()

        self.assertEqual(duration_ns(receipt, "admitted", "ready"), 20)
        self.assertEqual(
            run_metrics(receipt),
            {
                "image_resolve": 10,
                "launch_ready": 20,
                "admitted_to_command_finished": 50,
                "ready_to_command_finished": 30,
                "command": 20,
                "cleanup": 15,
                "request_total": 90,
            },
        )

    def test_missing_or_unknown_milestone_shapes_fail_closed(self) -> None:
        invalid_receipts = (
            {},
            {"milestones": []},
            {"milestones": None},
            {"milestones": ({"kind": "accepted", "elapsed_ns": 0},)},
            {"milestones": ["accepted"]},
            {"milestones": [{"elapsed_ns": 0}]},
            {"milestones": [{"kind": "accepted"}]},
            {
                "milestones": [
                    {"kind": "accepted", "elapsed_ns": 0, "unexpected": True}
                ]
            },
            {"milestones": [{"kind": "unknown", "elapsed_ns": 0}]},
        )
        for receipt in invalid_receipts:
            with self.subTest(receipt=receipt):
                with self.assertRaises(ValueError):
                    parse_milestones(receipt)

    def test_invalid_elapsed_values_fail_closed(self) -> None:
        for value in (-1, True, 1.5, "1"):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    parse_milestones(
                        {"milestones": [{"kind": "accepted", "elapsed_ns": value}]}
                    )

    def test_duplicate_out_of_order_and_regressing_milestones_fail_closed(self) -> None:
        invalid_sequences = (
            [
                {"kind": "accepted", "elapsed_ns": 0},
                {"kind": "accepted", "elapsed_ns": 1},
            ],
            [
                {"kind": "accepted", "elapsed_ns": 0},
                {"kind": "admitted", "elapsed_ns": 1},
                {"kind": "workload_resolved", "elapsed_ns": 2},
            ],
            [
                {"kind": "accepted", "elapsed_ns": 1},
                {"kind": "workload_resolved", "elapsed_ns": 0},
            ],
        )
        for milestones in invalid_sequences:
            with self.subTest(milestones=milestones):
                with self.assertRaises(ValueError):
                    parse_milestones({"milestones": milestones})

    def test_duration_rejects_missing_or_reversed_boundaries(self) -> None:
        receipt = complete_run_receipt()

        with self.assertRaises(ValueError):
            duration_ns(receipt, "ready", "missing")
        with self.assertRaises(ValueError):
            duration_ns(receipt, "ready", "admitted")

    def test_run_metrics_requires_the_complete_one_shot_sequence(self) -> None:
        receipt = complete_run_receipt()
        milestones = receipt["milestones"]
        assert isinstance(milestones, list)
        del milestones[3]

        with self.assertRaises(ValueError):
            run_metrics(receipt)


if __name__ == "__main__":
    unittest.main()
