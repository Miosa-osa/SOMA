import os
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.runner.command import parse_arguments
from benchmarks.local_alpha.runner.config import canonical_scenario
from benchmarks.local_alpha.runner.identities import IdentityGenerator


class RunnerConfigurationTests(unittest.TestCase):
    def test_selects_exactly_one_canonical_scenario(self) -> None:
        scenario = canonical_scenario(
            "base-cli-one-shot-node-22-1vcpu-1024mib-10240mib-denied"
        )

        self.assertEqual(scenario.caller, "cli")
        with self.assertRaisesRegex(ValueError, "unknown canonical scenario"):
            canonical_scenario("invented-scenario")

    def test_command_requires_positive_repetitions_and_explicit_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = root / "target" / "release"
            release.mkdir(parents=True)
            soma = release / "soma"
            mcp = release / "soma-mcp"
            runtime = root / "runtime" / "container"
            runtime.parent.mkdir()
            for executable in (soma, mcp, runtime):
                executable.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
                executable.chmod(0o700)
            arguments = [
                "--scenario-id",
                "base-cli-one-shot-node-22-1vcpu-1024mib-10240mib-denied",
                "--repetitions",
                "2",
                "--soma-bin",
                os.fspath(soma),
                "--soma-mcp-bin",
                os.fspath(mcp),
                "--apple-runtime",
                os.fspath(runtime),
                "--result-dir",
                os.fspath(root / "results"),
                "--cache-state",
                "cached",
            ]

            config = parse_arguments(arguments)
            self.assertEqual(config.repetitions, 2)
            with self.assertRaises(SystemExit):
                parse_arguments([value if value != "2" else "0" for value in arguments])


class RunnerIdentityTests(unittest.TestCase):
    def test_identity_generator_retries_until_ids_are_exact_and_unique(self) -> None:
        values = iter(("1" * 32, "1" * 32, "2" * 32))
        identities = IdentityGenerator(lambda: next(values))

        self.assertEqual(identities.new(), "1" * 32)
        self.assertEqual(identities.new(), "2" * 32)


if __name__ == "__main__":
    unittest.main()
