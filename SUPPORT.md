# SOMA support

SOMA by MIOSA is a pre-alpha open-source project.
Community support is best effort and no response-time or production-support guarantee is currently offered.

## Where to ask

Use a GitHub issue for a reproducible defect, compatibility failure, documentation error, or focused feature proposal.
Use GitHub Discussions for design questions and general help when Discussions are enabled for the repository.
Use the private process in [SECURITY.md](SECURITY.md) for vulnerabilities.
Use the private process in [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for conduct concerns.

Do not use a public issue to share credentials, private hostnames, tenant data, proprietary guest images, crash dumps containing secrets, or embargoed vulnerability details.

## Information to include

- The exact SOMA commit or release identity.
- Host operating system, kernel, architecture, CPU model, microcode, and KVM availability.
- Whether the result came from Apple Silicon development or Ubuntu 24.04 x86_64 production-target validation.
- Generation identity and compatibility metadata with secrets removed.
- The public command and terminal receipt or typed fault.
- Relevant milestones and monotonic timings.
- The smallest safe reproduction.
- Expected and observed behavior.
- Whether the host cache, disk, network, and resource setup were cold, warm, or prepared.

## Support boundaries

Apple Silicon macOS can validate platform-neutral contract, parser, state-machine, and documentation behavior.
It cannot validate KVM, x86_64 vCPU state, Linux memory mapping, seccomp, namespaces, cgroups, TAP networking, or XFS reflinks.

The project does not currently provide production incident response, managed hosting, workload debugging, provider billing help, or support for arbitrary host kernels and architectures.
Questions about a product built on SOMA should go to that product's operator unless the issue reproduces at the SOMA interface.

## Performance questions

Attach raw samples and the metadata required by [docs/benchmark-contract.md](docs/benchmark-contract.md).
State whether the test measured a cold Generation build, cold-cache restore, warm-cache restore, prepared resources, a paused Machine, or an already-Ready Machine.
Internal VMM milestones and exact ComputeSDK Burst TTI are different measurements and must not be presented as interchangeable.
