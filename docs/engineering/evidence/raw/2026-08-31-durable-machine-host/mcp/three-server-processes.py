"""Two separate soma-mcp processes over one sandbox: launch in the first, use it in the second."""
import json, os, subprocess, sys, uuid, base64

binary = sys.argv[1]
instance = uuid.uuid4().hex

def session(calls):
    proc = subprocess.Popen([binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.DEVNULL, env=os.environ.copy())
    seq = 0
    def rpc(method, params=None):
        nonlocal seq
        seq += 1
        frame = {"jsonrpc": "2.0", "id": seq, "method": method}
        if params is not None:
            frame["params"] = params
        proc.stdin.write((json.dumps(frame) + "\n").encode()); proc.stdin.flush()
        while True:
            line = proc.stdout.readline()
            if not line:
                raise SystemExit("mcp server closed the stream")
            reply = json.loads(line)
            if reply.get("id") == seq:
                return reply
    rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "probe", "version": "0"}})
    proc.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); proc.stdin.flush()
    out = []
    for label, name, arguments in calls:
        reply = rpc("tools/call", {"name": name, "arguments": arguments})
        inner = json.loads(reply["result"]["content"][0]["text"])
        result = inner.get("result") or {}
        stdout = result.get("stdout")
        if isinstance(stdout, dict) and stdout.get("data"):
            stdout = base64.b64decode(stdout["data"]).decode()
        out.append((label, os.getpid(), proc.pid, inner.get("error"), result.get("state"),
                    result.get("execution"), stdout))
    proc.stdin.close(); proc.wait(timeout=60)
    return out

rows = []
rows += session([("launch", "soma_launch", {"image": "busybox:stable-musl", "instance_id": instance,
                  "backend": "kvm", "vcpu_count": 1, "memory_mib": 1024, "storage_mib": 2048})])
rows += session([("exec-write", "soma_exec", {"instance_id": instance, "backend": "kvm",
                  "executable": "/bin/sh",
                  "arguments": ["-c", "echo written-by-the-first-mcp-process > /tmp/two.txt; echo wrote"]})])
rows += session([("exec-read", "soma_exec", {"instance_id": instance, "backend": "kvm",
                  "executable": "/bin/cat", "arguments": ["/tmp/two.txt"]}),
                 ("destroy", "soma_destroy", {"instance_id": instance, "backend": "kvm"})])
print("instance", instance)
for label, _, server_pid, error, state, execution, stdout in rows:
    print(f"{label:11s} server_pid={server_pid} error={error} state={state} execution={execution} stdout={stdout!r}")
