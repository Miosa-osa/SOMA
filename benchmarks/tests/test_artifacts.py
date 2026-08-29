import base64
import json
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.artifacts import (
    MAXIMUM_RAW_RECORD_BYTES,
    ArtifactWriter,
    validate_artifact_directory,
)


class ArtifactTests(unittest.TestCase):
    def test_validation_rejects_a_raw_record_above_the_capture_bound(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "evidence"
            destination.mkdir()
            record = {
                "record_type": "run_metadata",
                "schema": "soma.local-alpha.raw.v1",
                "run_id": "run-1",
                "padding": "x" * MAXIMUM_RAW_RECORD_BYTES,
            }
            (destination / "raw.ndjson").write_text(
                json.dumps(record) + "\n", encoding="utf-8"
            )
            (destination / "summary.json").write_text(
                json.dumps(
                    {"schema": "soma.local-alpha.summary.v1", "run_id": "run-1"}
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ValueError, "bound"):
                validate_artifact_directory(destination)

    def test_writer_rejects_prohibited_content_hidden_in_base64(self) -> None:
        payloads = (
            str(Path.home() / "private.txt").encode(),
            b"/private/tmp/soma-local-alpha-state-secret",
            b'{"hardware_uuid":"secret"}',
        )
        with tempfile.TemporaryDirectory() as temporary:
            for index, payload in enumerate(payloads):
                with self.subTest(payload=payload):
                    with ArtifactWriter(Path(temporary) / f"evidence-{index}") as writer:
                        with self.assertRaises(ValueError):
                            writer.append(
                                {
                                    "record_type": "run_metadata",
                                    "schema": "soma.local-alpha.raw.v1",
                                    "run_id": "run-1",
                                    "data_base64": base64.b64encode(payload).decode("ascii"),
                                }
                            )

    def test_writer_retains_ndjson_and_summary_atomically(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "evidence"
            with ArtifactWriter(destination) as writer:
                writer.append(
                    {
                        "record_type": "run_metadata",
                        "schema": "soma.local-alpha.raw.v1",
                        "run_id": "run-1",
                    }
                )
                writer.append(
                    {
                        "record_type": "sample",
                        "schema": "soma.local-alpha.raw.v1",
                        "run_id": "run-1",
                        "sample_id": "sample-1",
                        "scenario_id": "cli-one-shot",
                        "accepted": True,
                        "duration_ns": 100,
                    }
                )
                writer.finish({"schema": "soma.local-alpha.summary.v1", "run_id": "run-1"})

            records = [
                json.loads(line)
                for line in (destination / "raw.ndjson").read_text(encoding="utf-8").splitlines()
            ]
            self.assertEqual([record["record_type"] for record in records], ["run_metadata", "sample"])
            self.assertTrue((destination / "summary.json").is_file())
            validate_artifact_directory(destination)

    def test_validation_rejects_duplicate_sample_ids(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "evidence"
            with ArtifactWriter(destination) as writer:
                writer.append(
                    {
                        "record_type": "run_metadata",
                        "schema": "soma.local-alpha.raw.v1",
                        "run_id": "run-1",
                    }
                )
                for _ in range(2):
                    writer.append(
                        {
                            "record_type": "sample",
                            "schema": "soma.local-alpha.raw.v1",
                            "run_id": "run-1",
                            "sample_id": "duplicate",
                            "scenario_id": "scenario",
                            "accepted": True,
                            "duration_ns": 1,
                        }
                    )
                with self.assertRaises(ValueError):
                    writer.finish(
                        {"schema": "soma.local-alpha.summary.v1", "run_id": "run-1"}
                    )

            self.assertFalse((destination / "summary.json").exists())

    def test_writer_validates_summary_identity_before_publication(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / "evidence"
            with ArtifactWriter(destination) as writer:
                writer.append(
                    {
                        "record_type": "run_metadata",
                        "schema": "soma.local-alpha.raw.v1",
                        "run_id": "run-1",
                    }
                )
                with self.assertRaisesRegex(ValueError, "identity"):
                    writer.finish(
                        {"schema": "soma.local-alpha.summary.v1", "run_id": "run-2"}
                    )

            self.assertFalse((destination / "summary.json").exists())

    def test_writer_rejects_hardware_identity_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            with ArtifactWriter(Path(temporary) / "evidence") as writer:
                with self.assertRaises(ValueError):
                    writer.append(
                        {
                            "record_type": "run_metadata",
                            "schema": "soma.local-alpha.raw.v1",
                            "run_id": "run-1",
                            "hardware_uuid": "secret",
                        }
                    )


if __name__ == "__main__":
    unittest.main()
