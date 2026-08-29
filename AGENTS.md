# Agent instructions

## Purpose

SOMA is a security-sensitive virtual machine monitor and launch runtime.
Correctness, isolation, deterministic recovery, and honest measurements take priority over feature count.

## Platform contract

The production target is Ubuntu 24.04 on x86_64 bare-metal hosts with KVM.
Apple Silicon macOS also has a development-only Virtualization.framework backend for real local OCI lifecycle conformance.
The portable library and command-line client target Linux, macOS, and Windows, while local engines remain capability-gated.
Intel macOS, Windows, or another unsupported local host must fail closed or use an explicitly configured remote SOMA engine.
Never describe a macOS test as proof of KVM, x86_64, namespace, cgroup, TAP, seccomp, reflink, pidfd, snapshot, density, or production-latency behavior.

## Architecture rules

- Design deep modules with small interfaces.
- Keep KVM types, file descriptors, snapshot encoding, and jail implementation behind internal seams.
- Do not create a generic `core` module or any other god module.
- A source file should stay below 500 physical lines unless it is generated, a fixture, or a third-party license.
- A module should own one cohesive policy or mechanism.
- Add an ADR before changing a public interface, process topology, snapshot compatibility rule, or trust assumption.
- Prefer one process per virtual machine.
- Keep the MIOSA adapter outside this repository so SOMA remains independently usable.

## Security rules

- Treat guest memory, device queues, snapshot state, launch specifications, and guest-agent messages as hostile input.
- Every `unsafe` block requires a local `SAFETY` explanation and a test or invariant that makes the explanation auditable.
- Arithmetic derived from guest-controlled values must be checked before conversion, allocation, slicing, or I/O.
- Compatibility checks fail closed.
- Never publish a virtual machine as ready before fresh identity, entropy, time, network, and authenticated first-command readiness are proven.
- Do not add secrets, production addresses, tenant identifiers, private repository references, or local absolute paths.

## Testing rules

- Test behavior through public interfaces.
- Develop one vertical slice at a time using red, green, and refactor.
- Run formatting, linting, unit tests, documentation tests, dependency policy, and architecture checks locally.
- Linux KVM tests require a real `/dev/kvm` runner and are a separate required gate.
- Benchmark end-to-end readiness and publish raw samples, failure counts, cache state, concurrency, and excluded work.

## Documentation rules

- Put each full sentence on its own physical line in substantial Markdown files.
- Use a plain hyphen instead of an em dash.
- Separate measured facts, implementation decisions, targets, and hypotheses.
- Never claim production safety or benchmark leadership without retained evidence.
