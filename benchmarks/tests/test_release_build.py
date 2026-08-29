import subprocess
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.build_release import build_release
from benchmarks.local_alpha.provenance import (
    RELEASE_BUILD_COMMAND,
    BuildManifest,
)


REVISION = "a" * 40


def _repository(root: Path) -> None:
    (root / "crates/demo/src").mkdir(parents=True)
    (root / "benchmarks/local_alpha").mkdir(parents=True)
    (root / "target/release").mkdir(parents=True)
    (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    (root / "Cargo.lock").write_text("# lock\n", encoding="utf-8")
    (root / "crates/demo/src/lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
    (root / "benchmarks/local_alpha/run.py").write_text("RUN = 1\n", encoding="utf-8")


def _write_outputs(root: Path, content: bytes = b"fresh") -> None:
    for name in ("soma", "soma-mcp"):
        output = root / "target/release" / name
        output.write_bytes(content)
        output.chmod(0o700)


class ReleaseBuildTests(unittest.TestCase):
    def test_runs_exact_build_after_discarding_stale_outputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            root = workspace / "checkout"
            root.mkdir()
            _repository(root)
            _write_outputs(root, b"stale")
            calls: list[tuple[tuple[str, ...], Path, bool]] = []

            def run(command, *, cwd, check):
                calls.append((tuple(command), cwd, check))
                self.assertFalse((root / "target/release/soma").exists())
                self.assertFalse((root / "target/release/soma-mcp").exists())
                _write_outputs(root)
                return subprocess.CompletedProcess(command, 0)

            destination = workspace / "manifest.json"
            manifest = build_release(
                root,
                destination,
                run_command=run,
                checkout_probe=lambda unused: (REVISION, True),
            )

            self.assertEqual(calls, [(RELEASE_BUILD_COMMAND, root, True)])
            self.assertEqual(BuildManifest.load(destination), manifest)
            self.assertEqual(manifest.build_argv, RELEASE_BUILD_COMMAND)

    def test_rejects_invalid_checkout_and_manifest_destinations_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            root = workspace / "checkout"
            root.mkdir()
            _repository(root)
            existing = workspace / "manifest.json"
            existing.write_text("reserved\n", encoding="utf-8")

            def unexpected(*args, **kwargs):
                raise AssertionError("Cargo must not run")

            def non_git(_root):
                raise ValueError("build manifest requires an exact Git checkout")

            with self.assertRaisesRegex(ValueError, "already exist"):
                build_release(
                    root,
                    existing,
                    run_command=unexpected,
                    checkout_probe=lambda unused: (REVISION, True),
                )
            with self.assertRaisesRegex(ValueError, "absolute"):
                build_release(
                    root,
                    Path("manifest.json"),
                    run_command=unexpected,
                    checkout_probe=lambda unused: (REVISION, True),
                )
            with self.assertRaisesRegex(ValueError, "outside the source checkout"):
                build_release(
                    root,
                    root / "inside-checkout.json",
                    run_command=unexpected,
                    checkout_probe=lambda unused: (REVISION, True),
                )
            with self.assertRaisesRegex(ValueError, "clean Git"):
                build_release(
                    root,
                    workspace / "dirty.json",
                    run_command=unexpected,
                    checkout_probe=lambda unused: (REVISION, False),
                )
            with self.assertRaisesRegex(ValueError, "exact Git"):
                build_release(
                    root,
                    workspace / "nongit.json",
                    run_command=unexpected,
                    checkout_probe=non_git,
                )

    def test_build_failure_or_missing_new_outputs_never_writes_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workspace = Path(temporary)
            root = workspace / "checkout"
            root.mkdir()
            _repository(root)
            failed = workspace / "failed.json"

            def fail(command, *, cwd, check):
                return subprocess.CompletedProcess(command, 1)

            with self.assertRaises(subprocess.CalledProcessError):
                build_release(
                    root,
                    failed,
                    run_command=fail,
                    checkout_probe=lambda unused: (REVISION, True),
                )
            self.assertFalse(failed.exists())

            _write_outputs(root, b"stale")
            stale = workspace / "stale.json"

            def false_success(command, *, cwd, check):
                return subprocess.CompletedProcess(command, 0)

            with self.assertRaisesRegex(ValueError, "fresh release output"):
                build_release(
                    root,
                    stale,
                    run_command=false_success,
                    checkout_probe=lambda unused: (REVISION, True),
                )
            self.assertFalse(stale.exists())


if __name__ == "__main__":
    unittest.main()
