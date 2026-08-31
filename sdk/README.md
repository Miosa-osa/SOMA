# SOMA SDKs

SOMA has a Rust library, a command line and an MCP server. None of those is
what an application developer reaches for. Every other sandbox provider ships a
Python and a JavaScript SDK, and that is how anyone actually uses one.

This directory closes half of that gap.

```
sdk/
  python/   the Python SDK (this is real)
```

## What exists

`sdk/python` is a dependency-free Python package that drives the `soma` binary
through its stable `soma.cli.v1` JSON envelope. It follows the ComputeSDK
provider surface, because MIOSA is already a listed ComputeSDK provider and that
interface is the specification worth matching.

| Contract operation | Status | Backed by |
| --- | --- | --- |
| `sandbox.create` | supported | `soma machine launch` |
| `sandbox.getById` | supported | `soma machine inspect` |
| `sandbox.list` | **refused** | nothing; the CLI has no enumeration verb |
| `sandbox.destroy` | supported | `soma machine destroy` |
| `runCommand` | supported | `soma machine exec`, or `soma run` one-shot |
| `filesystem.readFile` | **refused** | nothing |
| `filesystem.writeFile` | **refused** | nothing |
| `filesystem.mkdir` | **refused** | nothing |
| `filesystem.readdir` | **refused** | nothing |
| `filesystem.exists` | **refused** | nothing |
| `filesystem.remove` | **refused** | nothing |

Beyond the contract, the SDK also exposes `stop()`, `inspect()`, `version()`
and `doctor()`, because the CLI has them and callers need them.

## What does not exist, and why it refuses

A refused operation raises `NotSupportedYet`, naming the capability. It does not
return an empty list, an empty string, or a `False`.

**`list()`.** The `soma` command line addresses every sandbox by its exact
32-character instance identity. There is no `soma machine list`. The SDK could
walk the durable state root and guess at its on-disk layout, but that layout is
private to the runtime, it would report sandboxes the runtime may no longer own,
and it would break the moment the state store changes. To make this supported,
the CLI needs an enumeration command that reports identities and states from the
state store it already owns.

**The whole `filesystem` surface.** This one is the closest to existing. The
guest control protocol in `crates/soma-guest` already defines every operation
the contract asks for: `FileRequest::Read`, `Write`, `MakeDirectory`,
`ReadDirectory`, `Exists` and `Remove`, with bounded paths and offsets. What is
missing is the last hop. The command line exposes no file subcommand and no
guest path argument on `run`, `launch` or `exec`, so nothing outside the crate
can reach that protocol.

The tempting emulation is to run `cat` and `base64` inside the guest. That was
rejected: it only works on images that ship those tools, it corrupts large or
binary payloads against the output limit, it is not atomic, and above all it
would report success through a path that has none of the protocol's bounds. The
fix is a CLI verb over the guest protocol that is already written, not a
workaround in Python.

An SDK that pretends is worse than one that refuses. A caller who gets
`NotSupportedYet` fixes their design in one minute. A caller who gets a
plausible empty list debugs the wrong system for a day.

## The JavaScript SDK

Not written. One SDK done well beats two done badly, and the second one is
mostly translation once the first has settled the hard questions: which
operations are honest, how the envelope decodes, where the refusals sit.

When it is written, it mirrors this package one-to-one:

* Same provider surface, in JavaScript casing: `create`, `getById`, `list`,
  `destroy`, `runCommand`, `filesystem.readFile` and the rest.
* Same refusals, as a `NotSupportedYet` error carrying the same `capability`
  string, so the two SDKs disagree about nothing.
* Same envelope decoding: one `soma --format json` process per operation,
  base64 output fields decoded to `Buffer`, the declared `byte_length`
  verified, an unknown `schema` rejected.
* Same treatment of a nonzero guest exit: a result carrying `exitCode`, not a
  thrown error, even though the CLI labels it `guest_nonzero`.
* Same seam for tests: the child-process call injected, so the suite runs with
  no KVM.
* Same absence of dependencies. Node's `child_process` is enough.

The Python package is the reference. Anything the JavaScript SDK would have to
invent is a signal that the CLI, not the SDK, is the thing to change.
