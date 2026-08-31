"""Envelope decoding, including the shapes that must be rejected."""

from __future__ import annotations

import base64
import json
import unittest

from soma import ProtocolError, Soma
from soma.envelope import exec_result, parse
from soma.process import Completed

from fake_cli import INSTANCE_ID, FakeCli, command_result, envelope, ok


class ParseTests(unittest.TestCase):
    def test_a_foreign_schema_is_rejected(self):
        document = json.dumps({"schema": "other.v9", "command": "run"}).encode()

        with self.assertRaises(ProtocolError):
            parse(document)

    def test_non_json_output_is_rejected(self):
        with self.assertRaises(ProtocolError):
            parse(b"soma: usage: command line validation failed")

    def test_a_silent_process_is_reported_with_its_stderr(self):
        fake = FakeCli([Completed(exit_code=127, stdout=b"", stderr=b"no such file")])
        soma = Soma("soma", runner=fake)

        with self.assertRaises(ProtocolError) as raised:
            soma.version()

        self.assertIn("no such file", str(raised.exception))


class OutputTests(unittest.TestCase):
    def test_captured_output_is_base64_decoded(self):
        parsed = parse(envelope("run", result=command_result(stdout=b"\x00\xffbytes")))

        self.assertEqual(exec_result(parsed).stdout, b"\x00\xffbytes")

    def test_a_length_that_contradicts_the_payload_is_rejected(self):
        result = command_result(stdout=b"hello")
        result["stdout"]["byte_length"] = 99

        with self.assertRaises(ProtocolError):
            exec_result(parse(envelope("run", result=result)))

    def test_output_that_is_not_base64_is_rejected(self):
        result = command_result(stdout=b"hello")
        result["stdout"]["data"] = "not base64!!"

        with self.assertRaises(ProtocolError):
            exec_result(parse(envelope("run", result=result)))

    def test_a_signal_terminal_status_reports_no_exit_code(self):
        result = command_result()
        result["execution"] = {"signaled": {"signal": 9}}

        decoded = exec_result(parse(envelope("run", result=result)))

        self.assertIsNone(decoded.exit_code)
        self.assertEqual(decoded.signal, 9)
        self.assertFalse(decoded.succeeded)

    def test_a_unit_variant_terminal_status_decodes_without_a_payload(self):
        result = command_result()
        result["execution"] = "timed_out"

        decoded = exec_result(parse(envelope("run", result=result)))

        self.assertEqual(decoded.status, "timed_out")
        self.assertIsNone(decoded.exit_code)


class VersionTests(unittest.TestCase):
    def test_version_returns_the_contract_report(self):
        report = {"version": "1.0.0-alpha.1", "envelope_schema": "soma.cli.v1"}
        soma = Soma("soma", runner=FakeCli([ok("version", report)]))

        self.assertEqual(soma.version()["envelope_schema"], "soma.cli.v1")

    def test_the_base64_helper_matches_the_cli_encoding(self):
        payload = b"hello\n"
        result = command_result(stdout=payload, instance_id=INSTANCE_ID)

        self.assertEqual(
            result["stdout"]["data"],
            base64.b64encode(payload).decode("ascii"),
        )


if __name__ == "__main__":
    unittest.main()
