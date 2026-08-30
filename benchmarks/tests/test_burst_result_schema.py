import json
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.burst.results import (
    LEGACY_RESULTS_SCHEMA,
    load_results,
    require_mergeable,
)
from benchmarks.tests.burst_fixtures import (
    metadata_record,
    plan,
    sample_record,
    write_results,
)


class BurstResultSchemaTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = Path(
            self.enterContext(tempfile.TemporaryDirectory(prefix="soma-schema-test-"))
        )

    def path(self, name: str = "results.jsonl") -> Path:
        return self.directory / name

    def test_malformed_engine_identity_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        metadata = metadata_record(declared)
        metadata["engine"]["generation_store"] = {
            "state": "configured",
            "locator_sha256": "not-a-digest",
        }
        path = write_results(
            self.path(), declared, [sample_record(declared, 1)], metadata=metadata
        )

        with self.assertRaisesRegex(ValueError, "generation_store identity is invalid"):
            load_results(path)

    def test_legacy_v1_results_remain_readable_without_engine_identity(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        path = write_results(self.path(), declared, [sample_record(declared, 1)])
        records = []
        for line in path.read_text(encoding="utf-8").splitlines():
            record = json.loads(line)
            record["schema"] = LEGACY_RESULTS_SCHEMA
            if record["record_type"] == "run_metadata":
                record.pop("engine")
            records.append(json.dumps(record))
        path.write_text("\n".join(records) + "\n", encoding="utf-8")

        results = load_results(path)

        self.assertNotIn("engine", results.metadata)

    def test_merging_different_engine_identities_fails_closed(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        first = load_results(
            write_results(self.path("a.jsonl"), declared, [sample_record(declared, 1)])
        )
        changed = metadata_record(declared)
        changed["engine"]["generation_store"] = {
            "state": "configured",
            "locator_sha256": "a" * 64,
        }
        second = load_results(
            write_results(
                self.path("b.jsonl"),
                declared,
                [sample_record(declared, 1)],
                metadata=changed,
            )
        )

        with self.assertRaisesRegex(ValueError, "different host identities"):
            require_mergeable([first, second])


if __name__ == "__main__":
    unittest.main()
