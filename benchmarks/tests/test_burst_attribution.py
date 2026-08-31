"""A run that does not score what it attempted has to say why in its own output."""

from __future__ import annotations

import io
import json
import unittest
from contextlib import redirect_stderr
from pathlib import Path

from benchmarks.local_alpha.burst.attribution import (
    breakdown_lines,
    failure_breakdown,
    process_detail,
)
from benchmarks.local_alpha.burst.slot import execute_slot
from benchmarks.local_alpha.burst.validation import validate_samples
from benchmarks.local_alpha.capture import ProcessCapture, StreamCapture

from benchmarks.tests.burst_fixtures import metadata_record, plan, sample_record


INSTANCE_ID = "4" * 32
OPERATION_IDS = {"launch": "1" * 32, "exec": "2" * 32, "destroy": "3" * 32}


def _stream(payload: bytes) -> StreamCapture:
    return StreamCapture(len(payload), payload, "0" * 64, False)


def _capture(stdout: bytes, *, exit_code: int, stderr: bytes = b"") -> ProcessCapture:
    return ProcessCapture(
        argv=("$SOMA_BIN",),
        exit_code=exit_code,
        duration_ns=7,
        harness_timed_out=False,
        stdout=_stream(stdout),
        stderr=_stream(stderr),
    )


def _refusal(code: str, message: str, *, retryable: bool = False) -> bytes:
    return json.dumps(
        {
            "schema": "soma.cli.v1",
            "command": "machine.launch",
            "status": "error",
            "result": None,
            "error": {"code": code, "message": message, "retryable": retryable},
            "receipt": None,
        }
    ).encode("utf-8")


class ProcessDetailTests(unittest.TestCase):
    def test_a_typed_refusal_is_named_by_code_and_exit_meaning(self) -> None:
        capture = _capture(
            _refusal("machine_not_hosted", "use `soma run` instead"), exit_code=76
        )

        detail = process_detail(
            {
                "exit_code": capture.exit_code,
                "harness_timed_out": False,
                "stderr": capture.stderr.as_dict(),
            },
            json.loads(capture.stdout.retained),
        )

        self.assertIn("exit=76(capability_unavailable)", detail)
        self.assertIn("code=machine_not_hosted", detail)
        self.assertIn("use `soma run` instead", detail)

    def test_a_silent_refusal_says_that_it_was_silent(self) -> None:
        detail = process_detail(
            {
                "exit_code": 74,
                "harness_timed_out": False,
                "stderr": _stream(b"").as_dict(),
            },
            None,
        )

        self.assertEqual(detail, "exit=74(backend_failure) stderr=empty")

    def test_a_process_that_never_started_names_the_spawn_error(self) -> None:
        self.assertEqual(
            process_detail({"spawn_error": "FileNotFoundError"}, None),
            "spawn_error=FileNotFoundError",
        )


class SlotAttributionTests(unittest.TestCase):
    def test_a_refused_launch_keeps_the_reason_the_command_line_printed(self) -> None:
        responses = {
            "launch": _capture(
                _refusal("machine_not_hosted", "the identity would not survive"),
                exit_code=76,
            ),
            "exec": _capture(b"", exit_code=66),
            "destroy": _capture(b"", exit_code=69),
        }

        def capture_process(argv, **_):
            return responses[argv[argv.index("machine") + 1]]

        sample = execute_slot(
            plan(),
            soma_binary=Path("/bin/soma"),
            state_root=Path("/state"),
            environment={},
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
            capture_process=capture_process,
            clock=lambda: 0,
        )

        self.assertFalse(sample.successful)
        reasons = {failure["reason"]: failure["detail"] for failure in sample.failures}
        self.assertIn("launch_process_failed", reasons)
        self.assertIn("code=machine_not_hosted", reasons["launch_process_failed"])
        self.assertIn("cleanup_failed", reasons)
        self.assertIn("exit=69(conflict)", reasons["cleanup_failed"])


class BreakdownTests(unittest.TestCase):
    def test_reasons_are_counted_with_the_details_that_explain_them(self) -> None:
        rows = failure_breakdown(
            [
                [
                    {
                        "reason": "launch_process_failed",
                        "operation": "launch",
                        "detail": "exit=76",
                    }
                ],
                [
                    {
                        "reason": "launch_process_failed",
                        "operation": "launch",
                        "detail": "exit=76",
                    }
                ],
                [
                    {
                        "reason": "cleanup_failed",
                        "operation": "destroy",
                        "detail": "exit=69",
                    }
                ],
            ]
        )

        self.assertEqual(rows[0]["reason"], "launch_process_failed")
        self.assertEqual(rows[0]["count"], 2)
        self.assertEqual(rows[0]["details"], [{"detail": "exit=76", "count": 2}])
        self.assertEqual([row["count"] for row in rows], [2, 1])
        self.assertEqual(breakdown_lines(rows)[0], "2x launch_process_failed at launch")


class AttributionContractTests(unittest.TestCase):
    def test_a_failure_that_names_no_cause_is_refused(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        sample = sample_record(
            declared,
            1,
            successful=False,
            failures=[
                {"reason": "launch_process_failed", "operation": "launch", "detail": ""}
            ],
        )

        with self.assertRaisesRegex(ValueError, "no attributable detail"):
            validate_samples(
                metadata_record(declared),
                [sample],
                {"attempted": 1, "failure_breakdown": []},
                require_attribution=True,
            )

    def test_a_completed_run_without_a_breakdown_is_refused(self) -> None:
        declared = plan(iterations=1, concurrency=1)

        with self.assertRaisesRegex(ValueError, "failure breakdown"):
            validate_samples(
                metadata_record(declared),
                [sample_record(declared, 1)],
                {"attempted": 1},
                require_attribution=True,
            )


class RelativePathTests(unittest.TestCase):
    def test_the_refusal_names_the_absolute_path_to_pass(self) -> None:
        from benchmarks.local_alpha.burst.command import main
        from benchmarks.tests.test_burst_command import CONTRACT_PROFILE

        argv = list(CONTRACT_PROFILE)
        argv[argv.index("/absolute/manifest.json")] = "manifest.json"
        stderr = io.StringIO()

        with redirect_stderr(stderr):
            with self.assertRaises(SystemExit):
                main(argv)

        printed = stderr.getvalue()
        self.assertIn("soma-burst: error:", printed)
        self.assertIn("'manifest.json' is relative", printed)
        self.assertIn(str(Path.cwd() / "manifest.json"), printed)
        self.assertNotIn("usage:", printed)


if __name__ == "__main__":
    unittest.main()
