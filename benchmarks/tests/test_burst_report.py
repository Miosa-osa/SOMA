import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.burst.report import generate
from benchmarks.local_alpha.burst.results import ResultsWriter

from benchmarks.tests.burst_fixtures import (
    complete_results,
    metadata_record,
    plan,
    sample_record,
    write_results,
)


class BurstReportTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = Path(
            self.enterContext(tempfile.TemporaryDirectory(prefix="soma-burst-doc-"))
        )

    def path(self, name: str) -> Path:
        return self.directory / name

    def test_a_complete_cohort_produces_an_evidence_shaped_document(self) -> None:
        path = complete_results(self.path("results.jsonl"))

        document = generate([path], title="Burst harness proof")

        self.assertTrue(document.startswith("# Burst harness proof - 2026-08-30\n"))
        for section in (
            "## Evidence boundary",
            "## Identities",
            "## Invocation",
            "## Measured boundary",
            "## Cohorts",
            "## Time to first command",
            "## Stage timings (ns)",
            "## Command output",
            "## Failures",
            "## Raw data",
            "## What this does not prove",
        ):
            self.assertIn(section, document)
        self.assertIn("nearest rank", document)
        self.assertIn("warm-cache-restore", document)
        self.assertIn("the image was pulled before the timer", document)
        self.assertIn("No sample failed", document)
        self.assertNotIn(chr(0x2013), document)
        self.assertNotIn(chr(0x2014), document)

    def test_a_container_backend_document_states_it_is_not_a_kvm_result(self) -> None:
        document = generate(
            [complete_results(self.path("results.jsonl"))], title="Proof"
        )
        boundary = document.split("## Identities")[0]

        self.assertIn("not a SOMA KVM performance result", boundary)
        self.assertIn("harness proof on the `docker` Backend", boundary)
        self.assertIn("not comparable to any provider benchmark", document)
        self.assertIn("no Generation digest", document)

    def test_a_kvm_backend_document_makes_no_harness_proof_claim(self) -> None:
        declared = plan(iterations=1, concurrency=1, backend="kvm")
        sample = sample_record(declared, 1)
        sample["observed"]["isolation"] = {
            "state": "observed",
            "value": "hardware_virtual_machine",
        }
        path = write_results(self.path("kvm.jsonl"), declared, [sample])

        document = generate([path], title="KVM")

        self.assertIn("SOMA KVM Backend with hardware virtual machine", document)
        self.assertNotIn("not a SOMA KVM performance result", document)

    def test_every_failure_is_listed_with_its_typed_reason(self) -> None:
        declared = plan(iterations=2, concurrency=1)
        path = write_results(
            self.path("results.jsonl"),
            declared,
            [
                sample_record(declared, 1),
                sample_record(declared, 2, successful=False),
            ],
        )

        document = generate([path], title="Proof")

        self.assertIn("command_unsuccessful", document)
        self.assertIn("repetition 2", document)
        self.assertIn("1 of 2", document)

    def test_an_incomplete_run_is_refused(self) -> None:
        declared = plan(iterations=2, concurrency=1)
        path = self.path("incomplete.jsonl")
        with ResultsWriter(path) as writer:
            writer.append(metadata_record(declared))
            writer.append(sample_record(declared, 1))

        with self.assertRaisesRegex(ValueError, "run is incomplete"):
            generate([path], title="Proof")

    def test_cohorts_from_different_experiment_classes_are_refused(self) -> None:
        warm = plan(iterations=1, concurrency=1)
        leased = plan(iterations=1, concurrency=1, experiment_class="ready-pool")
        first = write_results(
            self.path("warm.jsonl"), warm, [sample_record(warm, 1)]
        )
        second = write_results(
            self.path("ready.jsonl"), leased, [sample_record(leased, 1)]
        )

        with self.assertRaisesRegex(ValueError, "different experiment_class"):
            generate([first, second], title="Proof")

    def test_a_warm_class_without_recorded_preparation_is_refused(self) -> None:
        declared = plan(iterations=1, concurrency=1)
        metadata = metadata_record(declared)
        metadata["plan"]["prepared_before_timer"] = []
        path = write_results(
            self.path("results.jsonl"),
            declared,
            [sample_record(declared, 1)],
            metadata=metadata,
        )

        with self.assertRaisesRegex(ValueError, "prepared before the timer"):
            generate([path], title="Proof")

    def test_a_document_without_a_title_is_refused(self) -> None:
        path = complete_results(self.path("results.jsonl"))

        with self.assertRaisesRegex(ValueError, "requires a title"):
            generate([path], title="   ")

    def test_two_cohorts_of_one_class_merge_into_one_document(self) -> None:
        small = plan(iterations=2, concurrency=1)
        wide = plan(iterations=4, concurrency=4, image="node:22")
        first = complete_results(self.path("small.jsonl"), small)
        second = complete_results(self.path("wide.jsonl"), wide)

        document = generate([first, second], title="Ladder")

        self.assertIn("across 2 cohorts", document)
        self.assertIn("node-22-c4-n4", document)
        self.assertIn("busybox-stable-musl-c1-n2", document)


if __name__ == "__main__":
    unittest.main()
