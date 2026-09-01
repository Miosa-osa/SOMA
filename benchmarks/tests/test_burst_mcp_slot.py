import unittest

from benchmarks.local_alpha.burst.mcp_slot import MCP_BOUNDARY, execute_mcp_slot
from benchmarks.local_alpha.mcp_stdio import McpFrameCapture

from benchmarks.tests.burst_fixtures import CLEANUP_COMPLETE, encoded, plan


INSTANCE_ID = "4" * 32
OPERATION_IDS = {"launch": "1" * 32, "exec": "2" * 32, "destroy": "3" * 32}


class FakeSession:
    def call_tool(self, name, arguments):
        operation = name.removeprefix("soma_")
        receipt = {
            "milestones": [{"kind": "accepted", "elapsed_ns": 0}],
        }
        if operation == "launch":
            result = {"instance_id": INSTANCE_ID, "state": "ready"}
            receipt.update(
                {
                    "backend": "kvm",
                    "requested_shape": {
                        "vcpu_count": 1,
                        "memory_mib": 1024,
                        "storage_mib": 10240,
                    },
                    "effective_shape": {
                        "vcpu_count": {"state": "observed", "value": 1},
                        "memory_mib": {"state": "observed", "value": 1024},
                        "storage_mib": {"state": "observed", "value": 10240},
                    },
                }
            )
        elif operation == "exec":
            result = {
                "instance_id": INSTANCE_ID,
                "status": {"kind": "exited", "code": 0},
                "stdout": {
                    "encoding": "base64",
                    "byte_length": 10,
                    "data": encoded(b"v22.23.2\n")["data"],
                },
                "stderr": {"encoding": "base64", "byte_length": 0, "data": ""},
            }
        else:
            result = {"instance_id": INSTANCE_ID, "state": "destroyed"}
            receipt["cleanup"] = CLEANUP_COMPLETE
        response = {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "isError": False,
                "structuredContent": {
                    "schema": "soma.mcp.v1",
                    "operation": operation,
                    "operation_id": arguments["operation_id"],
                    "result": result,
                    "receipt": receipt,
                },
            },
        }
        return McpFrameCapture({}, response, 7)


class RefusingSession:
    def call_tool(self, name, arguments):
        operation = name.removeprefix("soma_")
        return McpFrameCapture(
            {},
            {
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "isError": True,
                    "structuredContent": {
                        "schema": "soma.mcp.v1",
                        "operation": operation,
                        "operation_id": arguments["operation_id"],
                        "error": {"code": "backend_unavailable"},
                    },
                },
            },
            7,
        )


class BurstMcpSlotTests(unittest.TestCase):
    def test_persistent_session_measures_launch_through_exec_and_cleans_up(self) -> None:
        ticks = iter(range(100, 180, 10))
        sample = execute_mcp_slot(
            plan(backend="kvm"),
            client=FakeSession(),
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
            clock=lambda: next(ticks),
        )

        self.assertTrue(sample.successful)
        self.assertEqual(sample.tti_ns, 30)
        self.assertEqual(sample.boundary, MCP_BOUNDARY)
        self.assertEqual(sample.observed["backend"], "kvm")
        self.assertEqual(sample.command["status"], "exited")
        self.assertEqual(sample.command["exit_code"], 0)
        self.assertEqual(sorted(sample.processes), ["destroy", "exec", "launch"])

    def test_tool_refusals_retain_their_typed_code(self) -> None:
        sample = execute_mcp_slot(
            plan(backend="kvm"),
            client=RefusingSession(),
            instance_id=INSTANCE_ID,
            operation_ids=OPERATION_IDS,
        )

        self.assertEqual(
            [(failure["operation"], failure["detail"]) for failure in sample.failures],
            [
                ("launch", "backend_unavailable"),
                ("destroy", "backend_unavailable"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
