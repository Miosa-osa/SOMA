import json
import os
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.provenance import (
    BuildManifest,
    build_child_environment,
    source_fingerprint,
    validate_release_build,
)


def _manifest_document() -> dict[str, object]:
    def binary(path: str, filename: str, digest: str) -> dict[str, object]:
        return {"path": path, "filename": filename, "sha256": digest, "size_bytes": 4}

    return {
        "schema": "soma.local-alpha.build.v2",
        "created_at_utc": "2026-08-28T12:00:00+00:00",
        "source_sha256": "1" * 64,
        "benchmark_sha256": "2" * 64,
        "git_revision": "a" * 40,
        "worktree_clean": True,
        "build_argv": ["cargo", "build", "--locked", "--release"],
        "binaries": {
            "soma": binary("$SOMA_BIN", "soma", "3" * 64),
            "soma_mcp": binary("$SOMA_MCP_BIN", "soma-mcp", "4" * 64),
        },
    }


class ProvenanceTests(unittest.TestCase):
    def test_external_build_manifest_round_trips_and_validates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = root / "target/release"
            release.mkdir(parents=True)
            (root / "crates/demo/src").mkdir(parents=True)
            (root / "benchmarks/local_alpha").mkdir(parents=True)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            (root / "crates/demo/src/lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            (root / "benchmarks/local_alpha/run.py").write_text("RUNNER = 1\n", encoding="utf-8")
            soma, mcp = release / "soma", release / "soma-mcp"
            soma.write_bytes(b"soma")
            mcp.write_bytes(b"mcp")
            soma.chmod(0o700)
            mcp.chmod(0o700)
            manifest = BuildManifest.create(
                root,
                soma,
                mcp,
                ["cargo", "build", "--locked", "--release"],
                git_revision="a" * 40,
                worktree_clean=True,
            )
            path = root / "build-manifest.json"

            manifest.write(path)
            loaded = BuildManifest.load(path)

            self.assertEqual(loaded, manifest)
            validate_release_build(root, loaded, soma, mcp)

    def test_manifest_loader_rejects_duplicate_json_keys(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "manifest.json"
            encoded = json.dumps(_manifest_document(), separators=(",", ":"))
            path.write_text('{"schema":"wrong",' + encoded[1:], encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "duplicate"):
                BuildManifest.load(path)

    def test_manifest_loader_rejects_non_utc_creation_time(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "manifest.json"
            document = _manifest_document()
            document["created_at_utc"] = "whenever"
            path.write_text(json.dumps(document), encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "fields"):
                BuildManifest.load(path)

    def test_source_fingerprint_changes_with_runtime_source_but_not_results(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "crates/demo/src").mkdir(parents=True)
            (root / "benchmarks/results").mkdir(parents=True)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            source = root / "crates/demo/src/lib.rs"
            source.write_text("pub fn value() -> u8 { 1 }\n", encoding="utf-8")

            before = source_fingerprint(root)
            (root / "benchmarks/results/run.json").write_text("{}\n", encoding="utf-8")
            self.assertEqual(source_fingerprint(root), before)
            source.write_text("pub fn value() -> u8 { 2 }\n", encoding="utf-8")
            self.assertNotEqual(source_fingerprint(root), before)

    def test_child_environment_excludes_secret_bearing_variables(self) -> None:
        source = {
            "HOME": "/private/home",
            "PATH": "/usr/bin:/bin",
            "TMPDIR": "/private/tmp",
            "LANG": "en_US.UTF-8",
            "ISORUN_API_KEY": "must-not-escape",
            "AWS_SECRET_ACCESS_KEY": "must-not-escape",
            "RANDOM_TOKEN": "must-not-escape",
        }

        clean = build_child_environment(source, {"SOMA_EXPLICIT_TEST_ROOT": "/tmp/state"})

        self.assertEqual(clean["HOME"], "/private/home")
        self.assertNotIn("ISORUN_API_KEY", clean)
        self.assertNotIn("AWS_SECRET_ACCESS_KEY", clean)
        self.assertNotIn("RANDOM_TOKEN", clean)
        self.assertEqual(clean["SOMA_EXPLICIT_TEST_ROOT"], "/tmp/state")

    def test_release_validation_rejects_stale_hashes_source_and_harness(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = root / "target/release"
            release.mkdir(parents=True)
            (root / "crates/demo/src").mkdir(parents=True)
            (root / "benchmarks/local_alpha").mkdir(parents=True)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            (root / "crates/demo/src/lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            harness = root / "benchmarks/local_alpha/run.py"
            harness.write_text("RUNNER = 1\n", encoding="utf-8")
            soma = release / "soma"
            mcp = release / "soma-mcp"
            soma.write_bytes(b"soma-release")
            mcp.write_bytes(b"mcp-release")
            soma.chmod(0o700)
            mcp.chmod(0o700)
            manifest = BuildManifest.create(
                root,
                soma,
                mcp,
                ["cargo", "build", "--release"],
                git_revision="a" * 40,
                worktree_clean=True,
            )

            validate_release_build(root, manifest, soma, mcp)
            harness.write_text("RUNNER = 2\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "harness"):
                validate_release_build(root, manifest, soma, mcp)

            harness.write_text("RUNNER = 1\n", encoding="utf-8")
            soma.write_bytes(b"stale")
            with self.assertRaises(ValueError):
                validate_release_build(root, manifest, soma, mcp)

            soma.write_bytes(b"soma-release")
            (root / "crates/demo/src/lib.rs").write_text("pub fn changed() {}\n", encoding="utf-8")
            with self.assertRaises(ValueError):
                validate_release_build(root, manifest, soma, mcp)

    def test_build_manifest_json_contains_no_absolute_binary_paths(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            release = root / "target/release"
            release.mkdir(parents=True)
            (root / "crates/demo/src").mkdir(parents=True)
            (root / "benchmarks/local_alpha").mkdir(parents=True)
            (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
            (root / "crates/demo/src/lib.rs").write_text("pub fn ok() {}\n", encoding="utf-8")
            (root / "benchmarks/local_alpha/run.py").write_text(
                "RUNNER = 1\n", encoding="utf-8"
            )
            soma = release / "soma"
            mcp = release / "soma-mcp"
            soma.write_bytes(b"soma")
            mcp.write_bytes(b"mcp")
            manifest = BuildManifest.create(
                root,
                soma,
                mcp,
                ["cargo", "build"],
                git_revision="a" * 40,
                worktree_clean=True,
            )

            encoded = json.dumps(manifest.as_dict())

            self.assertNotIn(str(root), encoded)
            self.assertNotIn(os.fspath(Path.home()), encoded)
            self.assertIn("$SOMA_BIN", encoded)
            self.assertIn("$SOMA_MCP_BIN", encoded)


if __name__ == "__main__":
    unittest.main()
