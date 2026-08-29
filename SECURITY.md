# SOMA security policy

## Pre-alpha warning

SOMA is pre-alpha research and implementation work.
It is not yet safe for untrusted production workloads.
No released version is currently designated as production-supported.

Do not use SOMA to execute hostile tenant code, protect valuable credentials, or provide a security boundary for production data until the project publishes an explicit supported release and independent security evidence.

## Reporting a vulnerability

Report suspected vulnerabilities through the repository's [private vulnerability reporting form](https://github.com/Miosa-osa/SOMA/security/advisories/new).
The repository must keep private vulnerability reporting enabled while it is public.
Do not include exploit details, secrets, guest images, crash dumps, or private host data in a public issue.

Include the following information when possible:

- The affected commit, tag, architecture, host kernel, CPU class, and configuration.
- The threat actor and trust assumption that the issue violates.
- Reproduction steps with the smallest safe proof of concept.
- Expected and observed behavior.
- Whether exploitation crosses a guest, Instance, process, network, artifact, or operator seam.
- Crash logs or traces with secrets and tenant data removed.
- Any known workaround or containment action.

Do not test against systems, data, Machines, or hosts that you do not own or have explicit permission to assess.
Stop testing if it risks persistence, lateral movement, data loss, service disruption, or exposure of another tenant.

## Disclosure process

Maintainers will attempt to acknowledge a well-formed report when an authorized maintainer is available.
Pre-alpha does not carry a response-time, remediation-time, bounty, or embargo guarantee.
The reporter and maintainers should agree on a disclosure date after impact, fix availability, downstream coordination, and release timing are understood.

Maintainers may request additional reproduction evidence, a draft advisory, CVE coordination, or validation against a candidate fix.
Credit will be offered unless the reporter declines it or attribution would create additional risk.

## Security-sensitive areas

Reports are especially valuable in these areas:

- KVM setup, vCPU state restoration, and host memory mappings.
- Guest-controlled descriptor, length, offset, queue, and device processing.
- Generation provenance, artifact replacement, snapshot parsing, and compatibility checks.
- Cross-Instance memory, disk, network, identifier, entropy, or transport leakage.
- Guest-agent authentication, replay resistance, Repair ordering, and false Ready receipts.
- Jail setup, namespace isolation, capabilities, seccomp, cgroups, and file-descriptor inheritance.
- Resource ownership, ambiguous retries, PID reuse, rollback, and cleanup.
- Denial of service that escapes the declared Machine resource limits.
- Dependency, build, release, signing, SBOM, or provenance compromise.

The detailed intended model is in [docs/threat-model.md](docs/threat-model.md).
That document defines intended invariants and does not claim the pre-alpha implementation satisfies them.

## Supported versions

| Version                        | Production security support |
| ------------------------------ | --------------------------- |
| `main` and pre-alpha snapshots | No                          |

Security fixes may land on the default branch without backports until a supported release policy is published.
