# SOMA deployment validation report

## Decision

- Result: `PASS`, `FAIL`, or `BLOCKED`
- Highest completed evidence level:
- Production authorization: `NO` during pre-alpha
- Validated by:
- Validation start and end time:

## Release identity

- Repository: `Miosa-osa/SOMA`
- Merge commit:
- Worktree status:
- Release artifact:
- SHA-256 digest:
- Deployment identifier:
- Deployment timestamp:
- Previous deployment identity:

## Host identity

- Host identifier:
- Host ownership and test authorization:
- Distribution and release:
- Kernel:
- Architecture:
- Bare metal or nested:
- CPU model and feature class:
- Microcode:
- NUMA topology:
- Physical RAM:
- KVM device and access identity:
- Cgroup version:
- Artifact filesystem:
- Disk-head filesystem and reflink capability:
- Network topology:

## Generation identity

- OCI reference and digest:
- Guest kernel and command line digest:
- Root filesystem digest:
- Memory Artifact digest and size:
- Machine-state digest:
- Guest-agent identity:
- Compatibility fingerprint:
- Certification evidence:

Use `NOT IMPLEMENTED` rather than inventing Generation data when the deployed revision has no real restore path.

## Phase results

| Phase                         | Result | Evidence | Failure or blocker |
| ----------------------------- | ------ | -------- | ------------------ |
| 0. Measurement boundary       |        |          |                    |
| 1. Linux KVM host             |        |          |                    |
| 2. Storage and immutability   |        |          |                    |
| 3. Deployed revision          |        |          |                    |
| 4. One real restore           |        |          |                    |
| 5. Failure behavior           |        |          |                    |
| 6. Isolation and burst        |        |          |                    |
| 7. Exact ComputeSDK benchmark |        |          |                    |
| 8. Report publication         |        |          |                    |

## Contract evidence

- Launch request identity and structural replay evidence:
- Canonical request fingerprint, when an encoded protocol exists:
- Ordered Launch milestones:
- Ready receipt:
- No-op Execute terminal evidence:
- `node -v` Execute terminal evidence:
- Stop receipt:
- Repeated Stop result:
- Idempotent replay result:
- Operation-conflict result:

## Isolation evidence

- Private memory write test:
- Private disk-head write test:
- Machine identity uniqueness:
- Network identity uniqueness:
- Transport and authentication uniqueness:
- Cross-Instance replay rejection:
- Resource ownership and cleanup:

## Performance evidence

- Experiment class:
- Runner location and network path:
- Cache and preparation state:
- Iterations and concurrency:
- Successes and failures:
- Cleanup successes and failures:
- Median:
- p95:
- p99:
- Wall time:
- Raw sample location:
- Complete log location:

## Failed and skipped checks

List every failed, skipped, unsupported, or unavailable check with its exact reason.
A skipped KVM check is not a passing KVM check.

## Security observations

Record any violation or uncertainty involving Generation integrity, private mappings, guest-controlled input, authentication, Repair, Ready, process containment, or cleanup.

## Containment and rollback

- Admission stopped at:
- Instances stopped:
- Evidence preserved at:
- Rollback identity:
- Rollback verification result:

## Conclusion

State only what the evidence proves.
Separate measured results from engineering targets and future implementation plans.
