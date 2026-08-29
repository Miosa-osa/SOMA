import hashlib
import json
import sys
import unittest

from benchmarks.local_alpha.mcp_stdio import McpStdioSession


FAKE_SERVER = r'''
import json
import sys

for line in sys.stdin:
    message = json.loads(line)
    if message.get("method") == "initialize":
        response = {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fixture", "version": "1"},
            },
        }
    elif message.get("method") == "tools/call":
        response = {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "content": [],
                "structuredContent": {
                    "schema": "soma.mcp.v1",
                    "operation": "run",
                    "result": {"instance_id": message["params"]["arguments"]["instance_id"]},
                },
            },
        }
    else:
        continue
    sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
    sys.stdout.flush()
'''

FAKE_STDERR_SERVER = r'''
import sys

sys.stderr.buffer.write(b"diagnostic-output")
sys.stderr.buffer.flush()
for _line in sys.stdin:
    pass
'''


class McpStdioTests(unittest.TestCase):
    def test_stderr_is_retained_bounded_and_hashed_after_close(self) -> None:
        payload = b"diagnostic-output"
        session = McpStdioSession(
            [sys.executable, "-c", FAKE_STDERR_SERVER],
            display_argv=["$SOMA_MCP_BIN"],
            environment={},
            response_timeout_seconds=5,
            maximum_stderr_bytes=8,
        )

        with session:
            pass

        capture = session.stderr_capture
        self.assertEqual(capture.retained, payload[:8])
        self.assertEqual(capture.observed_bytes, len(payload))
        self.assertEqual(capture.sha256, hashlib.sha256(payload).hexdigest())
        self.assertTrue(capture.truncated)

    def test_external_session_records_initialize_and_tool_frames(self) -> None:
        with McpStdioSession(
            [sys.executable, "-c", FAKE_SERVER],
            display_argv=["$SOMA_MCP_BIN"],
            environment={},
            response_timeout_seconds=5,
        ) as session:
            initialization = session.initialize("2024-11-05")
            call = session.call_tool(
                "soma_run",
                {"instance_id": "1" * 32},
            )

        self.assertEqual(initialization.response["result"]["protocolVersion"], "2024-11-05")
        self.assertEqual(call.request["method"], "tools/call")
        self.assertEqual(
            call.response["result"]["structuredContent"]["result"]["instance_id"],
            "1" * 32,
        )
        self.assertGreater(call.duration_ns, 0)
        self.assertEqual(call.clock, "time.perf_counter_ns")

    def test_concurrent_calls_are_correlated_by_json_rpc_id(self) -> None:
        from concurrent.futures import ThreadPoolExecutor

        with McpStdioSession(
            [sys.executable, "-c", FAKE_SERVER],
            display_argv=["$SOMA_MCP_BIN"],
            environment={},
            response_timeout_seconds=5,
        ) as session:
            session.initialize("2024-11-05")
            with ThreadPoolExecutor(max_workers=8) as pool:
                futures = [
                    pool.submit(
                        session.call_tool,
                        "soma_run",
                        {"instance_id": f"{index + 1:032x}"},
                    )
                    for index in range(8)
                ]
                captures = [future.result() for future in futures]

        returned = {
            capture.response["result"]["structuredContent"]["result"]["instance_id"]
            for capture in captures
        }
        self.assertEqual(returned, {f"{index + 1:032x}" for index in range(8)})


if __name__ == "__main__":
    unittest.main()
