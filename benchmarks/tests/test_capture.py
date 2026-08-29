import base64
import json
import sys
import unittest

from benchmarks.local_alpha.capture import run_external_process


class ExternalProcessCaptureTests(unittest.TestCase):
    def test_capture_retains_exact_binary_streams_and_monotonic_duration(self) -> None:
        capture = run_external_process(
            [
                sys.executable,
                "-c",
                "import os; os.write(1, b'\\xff\\x00'); os.write(2, b'err')",
            ],
            display_argv=["$PYTHON", "-c", "binary-fixture"],
            environment={},
            timeout_seconds=5,
            maximum_stream_bytes=1024,
        )
        document = capture.as_dict()

        self.assertEqual(capture.exit_code, 0)
        self.assertGreater(capture.duration_ns, 0)
        self.assertEqual(base64.b64decode(document["stdout"]["data_base64"]), b"\xff\x00")
        self.assertEqual(base64.b64decode(document["stderr"]["data_base64"]), b"err")
        self.assertEqual(document["argv"], ["$PYTHON", "-c", "binary-fixture"])
        self.assertEqual(document["clock"], "time.perf_counter_ns")

    def test_timeout_kills_the_child_and_is_recorded(self) -> None:
        capture = run_external_process(
            [sys.executable, "-c", "import time; time.sleep(10)"],
            display_argv=["$PYTHON", "-c", "timeout-fixture"],
            environment={},
            timeout_seconds=0.05,
            maximum_stream_bytes=1024,
        )

        self.assertTrue(capture.harness_timed_out)
        self.assertIsNotNone(capture.exit_code)
        self.assertLess(capture.duration_ns, 5_000_000_000)

    def test_capture_marks_oversized_stream_without_storing_unbounded_bytes(self) -> None:
        capture = run_external_process(
            [sys.executable, "-c", "import os; os.write(1, b'x' * 8192)"],
            display_argv=["$PYTHON", "-c", "oversize-fixture"],
            environment={},
            timeout_seconds=5,
            maximum_stream_bytes=128,
        )
        document = capture.as_dict()

        self.assertEqual(document["stdout"]["observed_bytes"], 8192)
        self.assertEqual(document["stdout"]["retained_bytes"], 128)
        self.assertTrue(document["stdout"]["truncated"])
        self.assertEqual(len(base64.b64decode(document["stdout"]["data_base64"])), 128)
        json.dumps(document)


if __name__ == "__main__":
    unittest.main()
