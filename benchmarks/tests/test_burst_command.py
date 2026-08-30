import contextlib
import io
import unittest

from benchmarks.local_alpha.burst.command import main, parser


CONTRACT_PROFILE = [
    "run",
    "--experiment-class",
    "warm-cache-restore",
    "--prepared",
    "the node:22 image was already present",
    "--backend",
    "kvm",
    "--image",
    "node:22",
    "--iterations",
    "100",
    "--concurrency",
    "100",
    "--build-manifest",
    "/absolute/manifest.json",
    "--soma-bin",
    "/absolute/target/release/soma",
    "--soma-mcp-bin",
    "/absolute/target/release/soma-mcp",
    "--results",
    "/absolute/results.jsonl",
    "--",
    "/usr/local/bin/node",
    "-v",
]


class BurstCommandTests(unittest.TestCase):
    def test_the_contract_profile_parses_with_its_workload_after_the_separator(
        self,
    ) -> None:
        arguments = parser().parse_args(CONTRACT_PROFILE)

        self.assertEqual(arguments.command, "run")
        self.assertEqual(arguments.iterations, 100)
        self.assertEqual(arguments.concurrency, 100)
        self.assertEqual(arguments.workload, ["--", "/usr/local/bin/node", "-v"])

    def test_an_unknown_experiment_class_is_rejected_by_the_parser(self) -> None:
        argv = list(CONTRACT_PROFILE)
        argv[2] = "steady-state"

        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                parser().parse_args(argv)

    def test_a_relative_input_path_is_refused(self) -> None:
        argv = list(CONTRACT_PROFILE)
        argv[argv.index("/absolute/manifest.json")] = "manifest.json"
        stderr = io.StringIO()

        with contextlib.redirect_stderr(stderr):
            with self.assertRaises(SystemExit):
                main(argv)

        self.assertIn("build manifest path must be absolute", stderr.getvalue())

    def test_a_workload_without_an_absolute_executable_is_refused(self) -> None:
        argv = [value for value in CONTRACT_PROFILE if value != "/usr/local/bin/node"]
        stderr = io.StringIO()

        with contextlib.redirect_stderr(stderr):
            with self.assertRaises(SystemExit):
                main(argv)

        self.assertIn("absolute guest executable", stderr.getvalue())

    def test_the_report_command_requires_results_a_title_and_an_output(self) -> None:
        arguments = parser().parse_args(
            [
                "report",
                "--results",
                "/a.jsonl",
                "--results",
                "/b.jsonl",
                "--title",
                "Proof",
                "--output",
                "/doc.md",
            ]
        )

        self.assertEqual([str(path) for path in arguments.results], ["/a.jsonl", "/b.jsonl"])
        self.assertEqual(arguments.title, "Proof")


if __name__ == "__main__":
    unittest.main()
