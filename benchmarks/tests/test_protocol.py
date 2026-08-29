import base64
import unittest

from benchmarks.local_alpha.matrix import Scenario, Shape, Workload
from benchmarks.local_alpha.protocol import (
    ProtocolValidationError,
    build_cli_calls,
    build_mcp_calls,
    validate_cli_response,
    validate_mcp_response,
)


INSTANCE_ID = "a" * 32
OPERATION_IDS = {
    "run": "1" * 32,
    "launch": "2" * 32,
    "exec": "3" * 32,
    "destroy": "4" * 32,
}


def scenario(
    caller: str,
    *,
    mode: str = "one_shot",
    network: str = "denied",
    outcome: str = "success",
    stdout_hex: str | None = "736f6d612d7265616479",
) -> Scenario:
    return Scenario(
        identifier=f"protocol-{caller}-{mode}-{network}-{outcome}".replace("_", "-"),
        kind="test",
        caller=caller,
        mode=mode,
        image="ubuntu:24.04",
        shape=Shape(2, 2_048, 4_096),
        network_policy=network,
        workload=Workload(
            name="protocol",
            executable="/bin/sh",
            arguments=("-c", "printf soma-ready"),
            expected_outcome=outcome,
            timeout_ms=123,
            maximum_output_bytes=456,
            expected_stdout_hex=stdout_hex,
        ),
    )


def encoded_stdout(value: bytes) -> dict[str, object]:
    return {
        "encoding": "base64",
        "byte_length": len(value),
        "data": base64.b64encode(value).decode("ascii"),
    }


def complete_receipt(operation_id: str) -> dict[str, object]:
    return {
        "instance_id": INSTANCE_ID,
        "operation_id": operation_id,
        "cleanup": {
            "method": "forced",
            "machine": "complete",
            "memory": "complete",
            "storage": "complete",
            "network": {
                "lease": "not_owned",
                "runtime_attachment": "complete",
                "address_leases": "complete",
                "egress_policy": "complete",
                "dns_policy": "complete",
                "proxy_policy": "not_owned",
                "ingress_bindings": "complete",
            },
            "guest_authority": "complete",
        },
    }


def cli_command_response(call, status: object, stdout: bytes = b"soma-ready") -> dict:
    return {
        "schema": "soma.cli.v1",
        "command": "run" if call.operation == "run" else "machine.exec",
        "status": "ok" if status == {"exited": {"code": 0}} else "error",
        "result": {
            "instance_id": INSTANCE_ID,
            "execution": status,
            "stdout": encoded_stdout(stdout),
            "stderr": encoded_stdout(b""),
        },
        "error": None,
        "receipt": complete_receipt(call.operation_id),
    }


def mcp_command_response(call, status: dict, stdout: bytes = b"soma-ready") -> dict:
    structured = {
        "schema": "soma.mcp.v1",
        "operation": call.operation,
        "operation_id": call.operation_id,
        "result": {
            "instance_id": INSTANCE_ID,
            "status": status,
            "stdout": encoded_stdout(stdout),
            "stderr": encoded_stdout(b""),
        },
        "receipt": complete_receipt(call.operation_id),
    }
    return {"jsonrpc": "2.0", "id": 7, "result": {"structuredContent": structured}}


class ProtocolTranslationTests(unittest.TestCase):
    def test_cli_network_labels_map_to_exact_egress_and_dns_flags(self) -> None:
        expected = {
            "denied": ("--egress", "denied", "--dns", "denied"),
            "unspecified": ("--egress", "unspecified", "--dns", "unspecified"),
            "allowed": (
                "--egress",
                "unrestricted",
                "--dns",
                "custom",
                "--dns-server",
                "1.1.1.1",
            ),
        }
        for label, flags in expected.items():
            with self.subTest(label=label):
                call = build_cli_calls(
                    scenario("cli", network=label),
                    soma_binary="/release/soma",
                    instance_id=INSTANCE_ID,
                    operation_ids=OPERATION_IDS,
                )[0]
                positions = tuple(value for value in call.argv if value in flags)
                self.assertEqual(positions, flags)

    def test_mcp_network_labels_map_to_exact_network_objects(self) -> None:
        expected = {
            "denied": {"egress": "denied", "dns": "denied"},
            "unspecified": {"egress": "unspecified", "dns": "unspecified"},
            "allowed": {
                "egress": "unrestricted",
                "dns": "custom",
                "dns_servers": ["1.1.1.1"],
            },
        }
        for label, network in expected.items():
            with self.subTest(label=label):
                call = build_mcp_calls(
                    scenario("mcp", network=label),
                    instance_id=INSTANCE_ID,
                    operation_ids=OPERATION_IDS,
                )[0]
                self.assertEqual(call.arguments["network"], network)

    def test_one_shot_and_managed_plans_preserve_exact_inputs(self) -> None:
        one_shot = build_cli_calls(
            scenario("cli"),
            soma_binary="/release/soma",
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
            global_arguments=("--state-root", "/state"),
        )
        managed = build_mcp_calls(
            scenario("mcp", mode="managed"),
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
        )
        managed_cli = build_cli_calls(
            scenario("cli", mode="managed"), soma_binary="soma",
            instance_id=INSTANCE_ID, operation_ids=OPERATION_IDS,
        )

        self.assertEqual([call.operation for call in one_shot], ["run"])
        self.assertEqual([call.operation for call in managed], ["launch", "exec", "destroy"])
        self.assertEqual([call.operation for call in managed_cli], ["launch", "exec", "destroy"])
        self.assertEqual([call.tool_name for call in managed], ["soma_launch", "soma_exec", "soma_destroy"])
        self.assertIn("--state-root", one_shot[0].argv)
        self.assertEqual(managed[0].arguments["vcpu_count"], 2)
        self.assertEqual(managed[1].arguments["arguments"], ["-c", "printf soma-ready"])
        self.assertEqual(managed[2].arguments["instance_id"], INSTANCE_ID)


class ProtocolValidationTests(unittest.TestCase):
    def test_cli_validates_each_expected_workload_outcome(self) -> None:
        cases = {
            "success": {"exited": {"code": 0}},
            "nonzero_exit": {"exited": {"code": 17}},
            "timeout": "timed_out",
            "output_limit": "output_limit_exceeded",
        }
        for outcome, status in cases.items():
            with self.subTest(outcome=outcome):
                item = scenario("cli", outcome=outcome, stdout_hex=None)
                call = build_cli_calls(
                    item,
                    soma_binary="soma",
                    instance_id=INSTANCE_ID,
                    operation_ids=OPERATION_IDS,
                )[0]
                evidence = validate_cli_response(
                    cli_command_response(call, status, b""),
                    scenario=item,
                    call=call,
                    instance_id=INSTANCE_ID,
                )
                self.assertEqual(evidence.outcome, outcome)
                self.assertTrue(evidence.cleanup_complete)

    def test_mcp_validates_wrapped_response_and_binary_stdout(self) -> None:
        item = scenario("mcp", stdout_hex="ff00fe0a")
        call = build_mcp_calls(
            item,
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
        )[0]
        evidence = validate_mcp_response(
            mcp_command_response(call, {"kind": "exited", "code": 0}, b"\xff\x00\xfe\n"),
            scenario=item,
            call=call,
            instance_id=INSTANCE_ID,
        )

        self.assertEqual(evidence.stdout_hex, "ff00fe0a")
        self.assertTrue(evidence.cleanup_complete)

    def test_mcp_validates_each_adverse_outcome(self) -> None:
        cases = {
            "nonzero_exit": {"kind": "exited", "code": 17},
            "timeout": {"kind": "timed_out"},
            "output_limit": {"kind": "output_limit_exceeded"},
        }
        for outcome, status in cases.items():
            item = scenario("mcp", outcome=outcome, stdout_hex=None)
            call = build_mcp_calls(
                item, instance_id=INSTANCE_ID, operation_ids=OPERATION_IDS,
            )[0]
            evidence = validate_mcp_response(
                mcp_command_response(call, status, b""), scenario=item,
                call=call, instance_id=INSTANCE_ID,
            )
            self.assertEqual(evidence.outcome, outcome)

    def test_managed_destroy_requires_complete_cleanup(self) -> None:
        item = scenario("mcp", mode="managed")
        call = build_mcp_calls(
            item,
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
        )[-1]
        response = {
            "schema": "soma.mcp.v1",
            "operation": "destroy",
            "operation_id": call.operation_id,
            "result": {"instance_id": INSTANCE_ID, "state": "destroyed"},
            "receipt": complete_receipt(call.operation_id),
        }
        response["receipt"]["cleanup"]["network"]["dns_policy"] = "incomplete"

        with self.assertRaisesRegex(ProtocolValidationError, "cleanup"):
            validate_mcp_response(
                response,
                scenario=item,
                call=call,
                instance_id=INSTANCE_ID,
            )

    def test_validation_rejects_wrong_schema_identity_outcome_and_stdout(self) -> None:
        item = scenario("cli")
        call = build_cli_calls(
            item,
            soma_binary="soma",
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
        )[0]
        mutations = (
            ("schema", lambda value: value.__setitem__("schema", "other")),
            ("instance", lambda value: value["result"].__setitem__("instance_id", "b" * 32)),
            ("outcome", lambda value: value["result"].__setitem__("execution", "timed_out")),
            ("stdout", lambda value: value["result"]["stdout"].__setitem__("data", "eA==")),
        )
        for label, mutate in mutations:
            with self.subTest(label=label):
                response = cli_command_response(call, {"exited": {"code": 0}})
                mutate(response)
                with self.assertRaises(ProtocolValidationError):
                    validate_cli_response(
                        response,
                        scenario=item,
                        call=call,
                        instance_id=INSTANCE_ID,
                    )


if __name__ == "__main__":
    unittest.main()
