# SOMA Python SDK

A small, dependency-free Python wrapper around the `soma` binary. It speaks the
stable `soma.cli.v1` JSON envelope and nothing else: no HTTP, no daemon, no
third-party packages, standard library only, Python 3.10 or newer.

The surface follows the ComputeSDK provider contract, because SOMA is meant to
be reachable through the same interface every other sandbox provider exposes.
Where the CLI cannot serve that contract yet, this SDK raises `NotSupportedYet`
naming the missing capability. It never emulates.

## Install

There is nothing to install. Put `sdk/python` on `PYTHONPATH`, or install it in
place:

```
pip install -e sdk/python
```

The `soma` binary must be on `PATH`, or you pass its path to `Soma(...)`.

## Worked example

This runs today against a `soma` built from this repository. It uses `version`
and `doctor`, which need no virtual machine, then shows the refusal that keeps
the SDK honest.

```python
from soma import NotSupportedYet, Soma

soma = Soma("./target/release/soma")

report = soma.version()
print(report["version"], report["envelope_schema"])

health = soma.doctor()
print(health["status"], health["backend"], health["reason"])

try:
    soma.list()
except NotSupportedYet as refusal:
    print("refused:", refusal.capability)
```

Output on a KVM host:

```
1.0.0-alpha.1 soma.cli.v1
probe_passed auto backend_probe_passed
refused: sandbox.list
```

Running a command needs a host whose backend can actually start a machine:

```python
from soma import Shape, Soma

soma = Soma("./target/release/soma")

# One command in a throwaway sandbox, with proven cleanup.
result = soma.run("alpine:3", ["/bin/echo", "hello"], timeout_ms=20_000)
print(result.exit_code, result.stdout_text)

# Or a durable sandbox you drive over several commands.
sandbox = soma.create("alpine:3", shape=Shape(vcpus=2, memory_mib=2048))
try:
    first = sandbox.run_command(["/bin/sh", "-c", "echo one"])
    second = sandbox.run_command(["/bin/false"])
    print(first.stdout_text, second.exit_code)
finally:
    sandbox.destroy()
```

## The API

`Soma(binary="soma", *, backend=None, state_root=None, runtime=None, runner=None)`

| Call | CLI it drives |
| --- | --- |
| `create(image, *, name, shape, instance_id, operation_id) -> Sandbox` | `soma machine launch` |
| `get_by_id(instance_id) -> Sandbox` | `soma machine inspect` |
| `list()` | refused |
| `destroy(instance_id) -> str` | `soma machine destroy` |
| `run(image, argv, ...) -> ExecResult` | `soma run` |
| `version() -> dict` | `soma version` |
| `doctor(strict=False) -> dict` | `soma doctor` |

`Sandbox`

| Call | CLI it drives |
| --- | --- |
| `run_command(argv, *, timeout_ms, max_output_bytes, operation_id) -> ExecResult` | `soma machine exec` |
| `inspect() -> Inspection` | `soma machine inspect` |
| `stop() -> str` | `soma machine stop` |
| `destroy() -> str` | `soma machine destroy` |
| `filesystem` | refused, every method |

`ExecResult` carries `stdout` and `stderr` as `bytes` (with `stdout_text` and
`stderr_text` for the lossy decoded form), `exit_code`, `signal`, `status`, and
the CLI's `receipt`. `Shape` carries `vcpus`, `memory_mib`, `storage_mib`,
`egress`, `dns`, `dns_servers` and `publish`; every field is optional so the
CLI's own defaults stay the single source of truth.

## What is refused, and why

* `list()` refuses because the command line has no enumeration verb at all.
  Every operation addresses one sandbox by its exact 32-character instance
  identity. Listing the state root directory from Python would be guessing at a
  private on-disk layout, and would report sandboxes the runtime may no longer
  own.
* Every `filesystem` method refuses because the CLI exposes no file subcommand
  and no guest path argument anywhere. The guest control protocol in
  `crates/soma-guest` already carries all six operations, so what is missing is
  the last hop from the command line to that protocol. Faking it by running
  `cat` and `base64` in the guest would work only on images that ship those
  tools, would corrupt payloads at the output limit, and would bypass every
  bound the real protocol enforces.

Both refusals raise `NotSupportedYet`, which is not a `SomaCliError`, so a
caller can tell "SOMA cannot do this" apart from "this attempt failed".

## Errors

| Exception | Raised when |
| --- | --- |
| `NotSupportedYet` | the contract asks for something the CLI cannot do |
| `SandboxNotFound` | code `machine_not_found` |
| `StateConflict` | code `state_conflict` |
| `GuestTimeout` | code `guest_timeout` |
| `OutputLimitExceeded` | code `output_limit` |
| `BackendUnavailable` | codes `backend_unavailable`, `unsupported_backend` |
| `SomaCliError` | any other envelope error |
| `ProtocolError` | the CLI printed no envelope, or an unreadable one |

A nonzero guest exit is a result, not an exception: the CLI reports it as the
error code `guest_nonzero`, and this SDK returns the `ExecResult` with that exit
code, because a caller running a command wants the code back.

## Tests

The `soma` binary is faked at the subprocess boundary, so the suite runs
anywhere, with no KVM and no OCI registry:

```
cd sdk/python && PYTHONPATH=.:tests python3 -m unittest discover -s tests -v
```
