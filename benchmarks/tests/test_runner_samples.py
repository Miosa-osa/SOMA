import base64
import json
import tempfile
import unittest
from pathlib import Path

from benchmarks.local_alpha.capture import ProcessCapture, StreamCapture
from benchmarks.local_alpha.mcp_stdio import McpFrameCapture
from benchmarks.local_alpha.runner.cli_sample import execute_cli_sample
from benchmarks.local_alpha.runner.config import canonical_scenario
from benchmarks.local_alpha.runner.identities import IdentityGenerator
from benchmarks.local_alpha.runner.mcp_sample import execute_mcp_sample


class _FatalSampleError(BaseException):
    pass


def _stream(value: bytes) -> StreamCapture:
    return StreamCapture(len(value), value, "0" * 64, False)


def _output(value: bytes = b"") -> dict[str, object]:
    return {
        "encoding": "base64",
        "byte_length": len(value),
        "data": base64.b64encode(value).decode("ascii"),
    }


def _result(operation: str, instance_id: str, *, mcp: bool) -> dict[str, object]:
    result: dict[str, object] = {"instance_id": instance_id}
    if operation in {"run", "exec"}:
        result.update(
            stdout=_output(b"soma-ready"),
            stderr=_output(),
        )
        result["status" if mcp else "execution"] = (
            {"kind": "exited", "code": 0} if mcp else {"exited": {"code": 0}}
        )
    else:
        result["state"] = "ready" if operation == "launch" else "destroyed"
    return result


def _receipt(operation: str, operation_id: str, instance_id: str) -> dict[str, object]:
    receipt: dict[str, object] = {
        "instance_id": instance_id,
        "operation_id": operation_id,
        "preparation": {"state": "observed", "value": "on_demand"},
    }
    if operation in {"run", "destroy"}:
        receipt["cleanup"] = "complete"
    return receipt


def _cli_response(operation: str, operation_id: str, instance_id: str) -> bytes:
    return json.dumps(
        {
            "schema": "soma.cli.v1",
            "command": "run" if operation == "run" else f"machine.{operation}",
            "status": "ok",
            "result": _result(operation, instance_id, mcp=False),
            "receipt": _receipt(operation, operation_id, instance_id),
        }
    ).encode()


def _mcp_response(operation: str, operation_id: str, instance_id: str) -> dict:
    envelope = {
        "schema": "soma.mcp.v1",
        "operation": operation,
        "operation_id": operation_id,
        "result": _result(operation, instance_id, mcp=True),
        "receipt": _receipt(operation, operation_id, instance_id),
    }
    return {"jsonrpc": "2.0", "id": 1, "result": {"structuredContent": envelope}}


def _cli_operation(argv) -> str:
    return "launch" if "launch" in argv else "exec" if "exec" in argv else "destroy"


class CliSampleTests(unittest.TestCase):
    scenario = canonical_scenario(
        "base-cli-managed-node-22-1vcpu-1024mib-10240mib-denied"
    )

    def test_managed_tti_ends_before_validation_and_excludes_destroy(self) -> None:
        values = iter(("1" * 32, "2" * 32, "3" * 32, "4" * 32))
        observed_display: list[tuple[str, ...]] = []

        def capture(argv, *, display_argv, **_kwargs) -> ProcessCapture:
            operation = _cli_operation(argv)
            operation_id = argv[argv.index("--operation-id") + 1]
            instance_id = argv[argv.index("--instance-id") + 1]
            stdout = _cli_response(operation, operation_id, instance_id)
            observed_display.append(tuple(display_argv))
            return ProcessCapture(
                tuple(display_argv), 0, 999, False, _stream(stdout), _stream(b"")
            )

        clock = iter((1_000, 1_040, 1_100))
        with tempfile.TemporaryDirectory() as temporary:
            outcome = execute_cli_sample(
                self.scenario,
                soma_binary=Path("/build/release/soma"),
                apple_runtime=Path("/runtime/container"),
                state_root=Path(temporary),
                environment={},
                identities=IdentityGenerator(lambda: next(values)),
                capture_process=capture,
                clock=lambda: next(clock),
            )

        self.assertTrue(outcome.accepted)
        self.assertEqual(outcome.duration_ns, 100)
        self.assertEqual(
            [item["operation"] for item in outcome.operations],
            ["launch", "exec", "destroy"],
        )
        self.assertIn("includes_inter_call", outcome.boundary)
        self.assertEqual(
            {item["preparation_class"] for item in outcome.operations},
            {"on_demand"},
        )
        self.assertTrue(all("$STATE_ROOT" in argv for argv in observed_display))

    def test_managed_destroy_is_attempted_after_base_exception(self) -> None:
        values = iter(("1" * 32, "2" * 32, "3" * 32, "4" * 32))
        operations: list[str] = []

        def capture(argv, *, display_argv, **_kwargs) -> ProcessCapture:
            operation = _cli_operation(argv)
            operations.append(operation)
            if operation == "exec":
                raise _FatalSampleError
            operation_id = argv[argv.index("--operation-id") + 1]
            instance_id = argv[argv.index("--instance-id") + 1]
            stdout = _cli_response(operation, operation_id, instance_id)
            return ProcessCapture(
                tuple(display_argv), 0, 1, False, _stream(stdout), _stream(b"")
            )

        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaises(_FatalSampleError):
                execute_cli_sample(
                    self.scenario,
                    soma_binary=Path("/build/release/soma"),
                    apple_runtime=Path("/runtime/container"),
                    state_root=Path(temporary),
                    environment={},
                    identities=IdentityGenerator(lambda: next(values)),
                    capture_process=capture,
                )
        self.assertEqual(operations, ["launch", "exec", "destroy"])


class _FakeMcpSession:
    def __init__(self) -> None:
        self.operations: list[str] = []

    def call_tool(self, name: str, arguments: dict[str, object]) -> McpFrameCapture:
        operation = name.removeprefix("soma_")
        self.operations.append(operation)
        if operation == "exec" and getattr(self, "fail_exec", False):
            raise _FatalSampleError
        response = _mcp_response(
            operation,
            str(arguments["operation_id"]),
            str(arguments["instance_id"]),
        )
        return McpFrameCapture({}, response, 999)


class McpSampleTests(unittest.TestCase):
    scenario = canonical_scenario(
        "base-mcp-managed-node-22-1vcpu-1024mib-10240mib-denied"
    )

    def test_managed_tti_ends_at_correlated_parse_and_excludes_destroy(self) -> None:
        values = iter(("1" * 32, "2" * 32, "3" * 32, "4" * 32))
        session = _FakeMcpSession()
        clock = iter((2_000, 2_050, 2_150))
        outcome = execute_mcp_sample(
            self.scenario,
            session=session,
            identities=IdentityGenerator(lambda: next(values)),
            clock=lambda: next(clock),
        )

        self.assertTrue(outcome.accepted)
        self.assertEqual(outcome.duration_ns, 150)
        self.assertEqual(session.operations, ["launch", "exec", "destroy"])
        self.assertIn("includes_inter_call", outcome.boundary)

    def test_managed_destroy_is_attempted_after_base_exception(self) -> None:
        values = iter(("1" * 32, "2" * 32, "3" * 32, "4" * 32))
        session = _FakeMcpSession()
        session.fail_exec = True
        with self.assertRaises(_FatalSampleError):
            execute_mcp_sample(
                self.scenario,
                session=session,
                identities=IdentityGenerator(lambda: next(values)),
            )
        self.assertEqual(session.operations, ["launch", "exec", "destroy"])


if __name__ == "__main__":
    unittest.main()
