import unittest
from pathlib import Path

from benchmarks.local_alpha.burst.slot import BurstSample, execute_slot
from benchmarks.local_alpha.capture import ProcessCapture, StreamCapture

from .burst_fixtures import CLEANUP_COMPLETE, encoded, envelope, plan


OPERATION_IDS = {
    "launch": "1" * 32,
    "exec": "2" * 32,
    "destroy": "3" * 32,
}
INSTANCE_ID = "4" * 32
NODE_OUTPUT = b"v22.23.2\n"


def _capture(stdout: bytes, *, exit_code: int = 0, timed_out: bool = False):
    empty = StreamCapture(0, b"", "0" * 64, False)
    return ProcessCapture(
        argv=("$SOMA_BIN",),
        exit_code=exit_code,
        duration_ns=7,
        harness_timed_out=timed_out,
        stdout=StreamCapture(len(stdout), stdout, "1" * 64, False),
        stderr=empty,
    )


def _launch(**overrides) -> bytes:
    body = {
        "result": {"state": "ready"},
        "milestones": [
            {"kind": "accepted", "elapsed_ns": 0},
            {"kind": "workload_resolved", "elapsed_ns": 10},
            {"kind": "admitted", "elapsed_ns": 20},
            {"kind": "machine_launched", "elapsed_ns": 30},
            {"kind": "ready", "elapsed_ns": 40},
        ],
        "receipt": {
            "backend": "docker_container",
            "isolation": {"state": "observed", "value": "linux_container"},
            "preparation": {"state": "observed", "value": "on_demand"},
            "workload": {"state": "resolved", "identity": {"generation_id": None}},
        },
    }
    body.update(overrides)
    return envelope("machine.launch", INSTANCE_ID, **body)


def _exec(code: int = 0, stdout: bytes = NODE_OUTPUT) -> bytes:
    return envelope(
        "machine.exec",
        INSTANCE_ID,
        result={
            "execution": {"exited": {"code": code}},
            "stdout": encoded(stdout),
            "stderr": encoded(b""),
        },
        milestones=[
            {"kind": "accepted", "elapsed_ns": 0},
            {"kind": "command_started", "elapsed_ns": 1},
            {"kind": "command_finished", "elapsed_ns": 500},
        ],
    )


def _destroy(cleanup: object = CLEANUP_COMPLETE) -> bytes:
    return envelope(
        "machine.destroy",
        INSTANCE_ID,
        result={"state": "destroyed"},
        milestones=[
            {"kind": "accepted", "elapsed_ns": 0},
            {"kind": "cleanup_started", "elapsed_ns": 1},
            {"kind": "cleanup_finished", "elapsed_ns": 900},
        ],
        receipt={"cleanup": cleanup},
    )


def _run(responses: dict[str, bytes | ProcessCapture]) -> BurstSample:
    ticks = iter((1_000, 1_500))

    def capture_process(argv, **_):
        operation = argv[argv.index("machine") + 1]
        response = responses[operation]
        return response if isinstance(response, ProcessCapture) else _capture(response)

    return execute_slot(
        plan(),
        soma_binary=Path("/bin/soma"),
        state_root=Path("/state"),
        environment={},
        instance_id=INSTANCE_ID,
        operation_ids=OPERATION_IDS,
        capture_process=capture_process,
        clock=lambda: next(ticks),
    )


class BurstSlotTests(unittest.TestCase):
    def test_a_complete_slot_records_timing_stages_and_exact_output(self) -> None:
        sample = _run(
            {"launch": _launch(), "exec": _exec(), "destroy": _destroy()}
        )

        self.assertTrue(sample.successful)
        self.assertEqual(sample.tti_ns, 500)
        self.assertTrue(sample.cleanup_complete)
        self.assertEqual(sample.failures, ())
        self.assertEqual(sample.command["exit_code"], 0)
        self.assertEqual(sample.command["stdout"]["byte_length"], len(NODE_OUTPUT))
        self.assertEqual(
            sample.command["stdout"]["data_base64"], encoded(NODE_OUTPUT)["data"]
        )
        self.assertEqual(sorted(sample.stages), ["destroy", "exec", "launch"])
        self.assertEqual(sample.observed["backend"], "docker_container")
        self.assertEqual(sample.stages["launch"][-1]["kind"], "ready")

    def test_a_nonzero_guest_command_is_retained_as_a_typed_failure(self) -> None:
        sample = _run(
            {"launch": _launch(), "exec": _exec(code=17), "destroy": _destroy()}
        )

        self.assertFalse(sample.successful)
        self.assertFalse(sample.command_succeeded)
        self.assertTrue(sample.cleanup_complete)
        self.assertEqual(sample.tti_ns, 500)
        self.assertEqual(
            sample.failures, ({"reason": "command_unsuccessful", "operation": "exec", "detail": "exited:17"},)
        )

    def test_a_failed_launch_still_destroys_and_records_its_reason(self) -> None:
        sample = _run(
            {
                "launch": _capture(b"", exit_code=74),
                "exec": _exec(),
                "destroy": _destroy(),
            }
        )

        self.assertFalse(sample.successful)
        self.assertIsNone(sample.tti_ns)
        self.assertIsNone(sample.command)
        self.assertTrue(sample.cleanup_complete)
        self.assertEqual(
            [failure["reason"] for failure in sample.failures],
            ["launch_process_failed"],
        )

    def test_a_launch_that_is_not_ready_is_rejected(self) -> None:
        sample = _run(
            {
                "launch": _launch(result={"state": "stopping"}),
                "exec": _exec(),
                "destroy": _destroy(),
            }
        )

        self.assertEqual(
            [failure["reason"] for failure in sample.failures],
            ["launch_response_invalid"],
        )

    def test_incomplete_cleanup_fails_the_sample_without_hiding_it(self) -> None:
        broken = dict(CLEANUP_COMPLETE, machine="incomplete")
        sample = _run(
            {"launch": _launch(), "exec": _exec(), "destroy": _destroy(broken)}
        )

        self.assertTrue(sample.command_succeeded)
        self.assertFalse(sample.cleanup_complete)
        self.assertFalse(sample.successful)
        self.assertEqual(
            [failure["reason"] for failure in sample.failures], ["cleanup_failed"]
        )

    def test_a_malformed_response_is_a_typed_failure_rather_than_a_crash(self) -> None:
        sample = _run(
            {
                "launch": _launch(),
                "exec": _capture(b"not json"),
                "destroy": _destroy(),
            }
        )

        self.assertEqual(
            [failure["reason"] for failure in sample.failures],
            ["command_response_invalid"],
        )

    def test_the_published_record_carries_the_class_and_the_boundary(self) -> None:
        sample = _run(
            {"launch": _launch(), "exec": _exec(), "destroy": _destroy()}
        )

        record = sample.as_record(
            plan=plan(), run_id="f" * 32, repetition=3, burst_index=1, slot_index=2
        )

        self.assertEqual(record["experiment_class"], "warm-cache-restore")
        self.assertEqual(record["sample_id"], "f" * 32 + "-000003")
        self.assertTrue(record["successful"])
        self.assertIn("excludes_destroy", record["boundary"])


if __name__ == "__main__":
    unittest.main()
