"""The refused half of the provider contract.

These tests exist to prove the SDK does not degrade silently. Each unsupported
operation must raise `NotSupportedYet` naming the capability, and must never
return a plausible looking value.
"""

from __future__ import annotations

import unittest

from soma import Filesystem, NotSupportedYet, Sandbox, Soma
from soma.process import Cli

from fake_cli import INSTANCE_ID, FakeCli

CONTRACT_FILESYSTEM_CALLS = [
    ("read_file", ("/etc/hostname",), {}),
    ("write_file", ("/tmp/a", b"payload"), {}),
    ("mkdir", ("/tmp/dir",), {"parents": True}),
    ("readdir", ("/tmp",), {}),
    ("exists", ("/tmp/a",), {}),
    ("remove", ("/tmp/a",), {"recursive": True}),
]


class ListRefusalTests(unittest.TestCase):
    def test_list_raises_instead_of_returning_an_empty_list(self):
        fake = FakeCli([])
        soma = Soma("soma", runner=fake)

        with self.assertRaises(NotSupportedYet) as raised:
            soma.list()

        self.assertEqual(raised.exception.capability, "sandbox.list")
        self.assertIn("enumeration", raised.exception.reason)
        self.assertEqual(fake.calls, [], "a refusal must not run the binary")

    def test_the_refusal_is_not_confused_with_a_cli_failure(self):
        from soma import SomaCliError

        soma = Soma("soma", runner=FakeCli([]))

        with self.assertRaises(NotSupportedYet) as raised:
            soma.list()

        self.assertNotIsInstance(raised.exception, SomaCliError)


class FilesystemRefusalTests(unittest.TestCase):
    def test_every_filesystem_call_raises_and_names_its_capability(self):
        filesystem = Filesystem(INSTANCE_ID)

        for name, arguments, keywords in CONTRACT_FILESYSTEM_CALLS:
            with self.subTest(call=name):
                with self.assertRaises(NotSupportedYet) as raised:
                    getattr(filesystem, name)(*arguments, **keywords)
                self.assertEqual(raised.exception.capability, f"filesystem.{name}")

    def test_a_sandbox_exposes_the_filesystem_surface_that_refuses(self):
        fake = FakeCli([])
        handle = Sandbox(INSTANCE_ID, Cli("soma", runner=fake))

        with self.assertRaises(NotSupportedYet):
            handle.filesystem.read_file("/etc/hostname")

        self.assertEqual(fake.calls, [], "a refusal must not run the binary")


if __name__ == "__main__":
    unittest.main()
