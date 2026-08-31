"""The supported half of the provider contract."""

from __future__ import annotations

import unittest

from soma import Cli, GuestTimeout, Sandbox, SandboxNotFound, Shape, Soma
from soma.envelope import ExecResult

from fake_cli import INSTANCE_ID, FakeCli, command_result, failed, ok


def client(*responses):
    """A provider whose CLI is scripted rather than executed."""

    fake = FakeCli(responses)
    return Soma("soma", state_root="/tmp/soma-state", runner=fake), fake


def sandbox(*responses):
    """A sandbox handle whose CLI is scripted rather than executed."""

    fake = FakeCli(responses)
    return Sandbox(INSTANCE_ID, Cli("soma", runner=fake)), fake


class CreateTests(unittest.TestCase):
    def test_create_launches_the_image_and_returns_the_instance_identity(self):
        soma, fake = client(
            ok("machine.launch", {"instance_id": INSTANCE_ID, "state": "ready"})
        )

        launched = soma.create("alpine:3", name="worker")

        self.assertEqual(launched.instance_id, INSTANCE_ID)
        self.assertEqual(
            fake.last_call,
            [
                "soma",
                "--format",
                "json",
                "--state-root",
                "/tmp/soma-state",
                "machine",
                "launch",
                "--name",
                "worker",
                "alpine:3",
            ],
        )

    def test_create_passes_only_the_shape_fields_the_caller_set(self):
        soma, fake = client(
            ok("machine.launch", {"instance_id": INSTANCE_ID, "state": "ready"})
        )

        soma.create("alpine:3", shape=Shape(vcpus=2, egress="internet"))

        self.assertIn("--vcpus", fake.last_call)
        self.assertEqual(fake.last_call[fake.last_call.index("--vcpus") + 1], "2")
        self.assertIn("--egress", fake.last_call)
        self.assertNotIn("--memory-mib", fake.last_call)


class RunCommandTests(unittest.TestCase):
    def test_run_command_returns_streams_and_exit_code(self):
        handle, fake = sandbox(
            ok("machine.exec", command_result(stdout=b"hello\n", exit_code=0))
        )

        result = handle.run_command(["/bin/echo", "hello"], timeout_ms=5000)

        self.assertIsInstance(result, ExecResult)
        self.assertEqual(result.stdout_text, "hello\n")
        self.assertEqual(result.exit_code, 0)
        self.assertTrue(result.succeeded)
        self.assertEqual(fake.last_call[-3:], ["--", "/bin/echo", "hello"])

    def test_a_nonzero_guest_exit_is_a_result_and_not_an_exception(self):
        handle, _ = sandbox(
            failed(
                "machine.exec",
                "guest_nonzero",
                "guest command did not exit successfully",
                exit_code=10,
                result=command_result(stderr=b"boom\n", exit_code=3),
            )
        )

        result = handle.run_command(["/bin/false"])

        self.assertEqual(result.exit_code, 3)
        self.assertEqual(result.stderr_text, "boom\n")
        self.assertFalse(result.succeeded)

    def test_a_guest_timeout_raises_rather_than_reporting_an_exit_code(self):
        handle, _ = sandbox(
            failed(
                "machine.exec",
                "guest_timeout",
                "guest command exceeded its deadline",
                exit_code=124,
                retryable=True,
            )
        )

        with self.assertRaises(GuestTimeout) as raised:
            handle.run_command(["/bin/sleep", "600"])

        self.assertTrue(raised.exception.retryable)

    def test_a_bare_string_command_is_refused_because_there_is_no_guest_shell(self):
        handle, _ = sandbox()

        with self.assertRaises(TypeError):
            handle.run_command("/bin/echo hello")


class LifecycleTests(unittest.TestCase):
    def test_get_by_id_inspects_first_so_an_unknown_identity_fails_early(self):
        soma, _ = client(
            failed(
                "machine.inspect",
                "machine_not_found",
                "sandbox instance was not found",
                exit_code=66,
            )
        )

        with self.assertRaises(SandboxNotFound):
            soma.get_by_id(INSTANCE_ID)

    def test_get_by_id_returns_a_handle_for_a_known_sandbox(self):
        soma, _ = client(
            ok(
                "machine.inspect",
                {
                    "instance_id": INSTANCE_ID,
                    "state": "ready",
                    "backend": "linux_kvm",
                },
            )
        )

        self.assertEqual(soma.get_by_id(INSTANCE_ID).instance_id, INSTANCE_ID)

    def test_destroy_reports_the_state_the_cli_proved(self):
        soma, fake = client(
            ok("machine.destroy", {"instance_id": INSTANCE_ID, "state": "destroyed"})
        )

        self.assertEqual(soma.destroy(INSTANCE_ID), "destroyed")
        self.assertIn("destroy", fake.last_call)

    def test_run_uses_the_one_shot_command_and_keeps_the_argv_separator(self):
        soma, fake = client(ok("run", command_result(stdout=b"ok\n")))

        result = soma.run("alpine:3", ["/bin/echo", "ok"], timeout_ms=20000)

        self.assertEqual(result.stdout_text, "ok\n")
        self.assertEqual(fake.last_call[-4:], ["alpine:3", "--", "/bin/echo", "ok"])
        self.assertIn("--timeout-ms", fake.last_call)


if __name__ == "__main__":
    unittest.main()
