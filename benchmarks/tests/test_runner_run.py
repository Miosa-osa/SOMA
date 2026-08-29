import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from benchmarks.local_alpha.capture import StreamCapture
from benchmarks.local_alpha.mcp_stdio import McpFrameCapture
from benchmarks.local_alpha.provenance import BuildManifest
from benchmarks.local_alpha.runner.model import SampleOutcome
from benchmarks.local_alpha.runner.run import run_benchmark
from benchmarks.tests.runner_fixtures import (
    build_manifest,
    make_config,
    sample_outcome,
)


class BenchmarkRunTests(unittest.TestCase):
    def test_run_finishes_valid_artifacts_without_recording_state_root(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = make_config(root)
            state_roots: list[str] = []

            def sample(scenario, *, state_root, identities, **_kwargs) -> SampleOutcome:
                state_roots.append(os.fspath(state_root))
                return sample_outcome(identities)

            manifest = build_manifest(config)
            with patch.object(
                BuildManifest,
                "create",
                side_effect=AssertionError("measured execution must not build"),
            ):
                summary = run_benchmark(
                    config,
                    cli_sampler=sample,
                    build_manifest=manifest,
                )

            raw = (root / "results" / "raw.ndjson").read_text(encoding="utf-8")
            self.assertTrue(summary["all_samples_accepted"])
            self.assertEqual(summary["metrics"]["external_tti"]["accepted_count"], 2)
            self.assertEqual(len(set(state_roots)), 1)
            self.assertNotIn(state_roots[0], raw)
            self.assertIn("$STATE_ROOT", raw)

    def test_unvalidated_cleanup_halts_further_admission(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = make_config(root, repetitions=5)
            admitted: list[int] = []

            def sample(scenario, *, identities, **_kwargs) -> SampleOutcome:
                admitted.append(len(admitted) + 1)
                return sample_outcome(
                    identities,
                    accepted=False,
                    cleanup_validated=False,
                )

            summary = run_benchmark(
                config,
                cli_sampler=sample,
                build_manifest=build_manifest(config),
            )

            self.assertEqual(admitted, [1])
            self.assertEqual(summary["attempted_repetitions"], 1)
            self.assertFalse(summary["all_samples_accepted"])

    def test_mixed_preparation_fails_before_another_sample_is_admitted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = make_config(root, repetitions=4)
            preparations = iter(("on_demand", "prepared_worker", "ready_lease"))
            admitted: list[str] = []

            def sample(scenario, *, identities, **_kwargs) -> SampleOutcome:
                preparation = next(preparations)
                admitted.append(preparation)
                value = sample_outcome(identities)
                return SampleOutcome(
                    instance_id=value.instance_id,
                    operation_ids=value.operation_ids,
                    duration_ns=value.duration_ns,
                    boundary=value.boundary,
                    accepted=value.accepted,
                    cleanup_validated=value.cleanup_validated,
                    operations=({"preparation_class": preparation},),
                    receipt_metrics_ns=value.receipt_metrics_ns,
                    errors=value.errors,
                )

            with self.assertRaisesRegex(ValueError, "preparation"):
                run_benchmark(
                    config,
                    cli_sampler=sample,
                    build_manifest=build_manifest(config),
                )

            self.assertEqual(admitted, ["on_demand", "prepared_worker"])

    def test_mcp_uses_one_persistent_session_for_all_samples(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            config = make_config(root, caller="mcp", repetitions=3)
            sessions: list[object] = []
            sampled_sessions: list[object] = []

            class Session:
                def __init__(self, *_args, **kwargs) -> None:
                    self.display_argv = kwargs["display_argv"]
                    self.exit_code = None
                    sessions.append(self)

                def __enter__(self):
                    return self

                def __exit__(self, *_args) -> None:
                    self.exit_code = 0
                    self.stderr_capture = StreamCapture(
                        observed_bytes=4,
                        retained=b"safe",
                        sha256="a" * 64,
                        truncated=False,
                    )
                    return None

                def initialize(self, _version: str) -> McpFrameCapture:
                    return McpFrameCapture({}, {"result": {}}, 1)

            def sample(scenario, *, session, identities) -> SampleOutcome:
                sampled_sessions.append(session)
                return sample_outcome(identities, duration_ns=11)

            run_benchmark(
                config,
                mcp_sampler=sample,
                session_factory=Session,
                build_manifest=build_manifest(config),
            )

            self.assertEqual(len(sessions), 1)
            self.assertEqual(sampled_sessions, [sessions[0]] * 3)
            self.assertEqual(sessions[0].display_argv[-1], "$STATE_ROOT")
            records = [
                json.loads(line)
                for line in (root / "results/raw.ndjson").read_text(
                    encoding="utf-8"
                ).splitlines()
            ]
            process_records = [
                record
                for record in records
                if record["record_type"] == "mcp_process"
            ]
            self.assertEqual(len(process_records), 1)
            self.assertEqual(process_records[0]["stderr"]["data_base64"], "c2FmZQ==")


if __name__ == "__main__":
    unittest.main()
