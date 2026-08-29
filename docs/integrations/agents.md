# Agent integration

SOMA exposes a local stdio Model Context Protocol server through the `soma-mcp` binary.
The protocol adapter is bounded and tested, and `soma-mcp` now serves its tools through the shared `soma-local` runtime adapter.
The exported `UnavailableRuntime` remains the fail-closed runtime for an unsupported Host, so an execution tool reports an unsupported Backend instead of pretending that a sandbox ran.

## Build

Build the binary from the [SOMA repository](https://github.com/Miosa-osa/SOMA):

```sh
cargo build --release -p soma-mcp
```

Use the absolute path to `target/release/soma-mcp` in client configuration.
The stdio MCP transport requires no API key and has no remote MCP endpoint.
The selected backend may contact a configured OCI registry while resolving an image.
Do not put secrets in MCP arguments because tool inputs can be retained in an agent transcript.

## Client setup

These examples reflect the locally installed CLI behavior as of August 28, 2026.
Select an explicit Backend, because an unsupported local engine fails closed rather than downgrading.

### Claude Code

```sh
claude mcp add --scope user soma -- /absolute/path/to/soma-mcp
```

Claude Code defaults this command form to stdio transport.

### Codex

```sh
codex mcp add soma -- /absolute/path/to/soma-mcp
```

### OSA

The current OSA installation has no MCP registration subcommand.
Add this entry under `mcpServers` in `~/.osa/mcp.json`:

```json
{
  "mcpServers": {
    "soma": {
      "command": "/absolute/path/to/soma-mcp",
      "args": [],
      "autoConnect": true,
      "description": "Local SOMA sandbox tools"
    }
  }
}
```

SOMA accepts OSA's MCP `2024-11-05` initialize request as well as the current MCP `2026-07-28` request through `rmcp` protocol negotiation.

### Hermes Agent

```sh
hermes mcp add soma --command /absolute/path/to/soma-mcp
hermes mcp test soma
```

No `--env` values are required.

## Tool surface

| Tool | Purpose | Effect hint |
| --- | --- | --- |
| `soma_doctor` | Probe support and runtime readiness without claiming production readiness. | Read-only |
| `soma_run` | Launch a fresh VM, execute one direct command, collect evidence, and clean up. | Mutating and potentially open-world |
| `soma_launch` | Launch a managed VM from an OCI image. | Mutating |
| `soma_exec` | Execute one direct command in a managed VM. | Mutating and potentially open-world |
| `soma_inspect` | Read managed lifecycle state and the latest receipt. | Read-only |
| `soma_stop` | Stop a managed VM and return cleanup evidence. | Destructive lifecycle transition |
| `soma_destroy` | Destroy a managed VM and require cleanup evidence. | Destructive lifecycle transition |

Every lifecycle and execution tool except `soma_doctor` accepts an optional `operation_id` for receipt correlation and retry control.
`soma_run` and `soma_launch` also accept an optional `instance_id`.
Each ID must contain exactly 32 nonzero lowercase hexadecimal characters.
SOMA generates a UUIDv4 simple-form ID when an optional ID is omitted and returns the generated ID in the structured response.
Supplying the same IDs on a retry preserves retry intent, but tool annotations do not make a blanket idempotency claim because IDs may be omitted.

## Reproducible machine customization

`soma_run` and `soma_launch` use the same machine-shape contract as the portable facade.

| Field | Default | Portable request bound |
| --- | ---: | ---: |
| `vcpu_count` | 1 | 1 through 65,535 |
| `memory_mib` | 1,024 | 1 through 18,446,744,073,709,551,615 |
| `storage_mib` | 10,240 | 1 through 18,446,744,073,709,551,615 |

These are numeric validation bounds rather than capacity promises.
The selected backend performs separate capability and admission checks.
The development Apple backend may explicitly report requested root storage as unavailable, while a certified production backend must enforce every required shape dimension or fail closed.
The OCI `image` reference selects the software customization input and is limited to 1,024 bytes without a URL scheme, whitespace, NUL, backslash, or a leading dash.
Reproducibility comes from the exact resolved platform-manifest digest recorded in the execution receipt, not from a potentially mutable image tag.
The optional `display_name` is metadata only and never replaces the canonical `instance_id`.
It is 1 to 63 bytes of lowercase ASCII letters, digits, or hyphens, with an alphanumeric first and last byte.

`network_policy` is an explicit three-state request on `soma_run` and `soma_launch`:

| Value | Meaning |
| --- | --- |
| `unspecified` | Default. The request makes no network-access guarantee, and the receipt records the available observation. |
| `denied` | Require guest network access to be denied. |
| `allowed` | Require guest network access to be allowed. |

A backend that cannot enforce an explicit `denied` or `allowed` policy must fail closed.

## Direct command and output contract

Commands are an absolute guest `executable` plus an `arguments` array.
SOMA never accepts a shell command string, ambient host executable selection, environment map, mount list, runtime path, secret field, or remote MCP URL.
The executable is limited to 4,096 bytes.
There may be at most 4,096 arguments, each argument is limited to 128 KiB, and the total executable plus argument payload is limited to 1 MiB.
NUL bytes are rejected.

`timeout_ms` defaults to 30,000 and must be from 1 through 86,400,000.
`max_output_bytes` defaults to 1,048,576 and must be from 1 through 16,777,216 across stdout and stderr combined.
Raw stdout and stderr remain binary and are returned as base64 objects with explicit byte lengths.
The execution receipt is returned as bounded structured JSON and is never reconstructed from display text.

## Operational boundaries

The MCP process reserves stdout exclusively for newline-delimited JSON-RPC.
Diagnostics go to stderr.
An inbound MCP message above 8 MiB terminates the session before runtime admission.
The server admits at most 32 concurrent tool executions and rejects excess work instead of building an unbounded queue.

macOS is a development-only backend for portability testing.
Production isolation targets Linux with KVM support, and unsupported targets fail closed.
Backend state must live behind the shared thread-safe `ToolRuntime` adapter and durable SOMA store rather than in the MCP server process.
Restarting `soma-mcp` must reconnect to durable lifecycle state through that adapter and must not imply that a managed VM was destroyed or forgotten.
