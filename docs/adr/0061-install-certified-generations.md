# ADR 0061 - Install certified Generations from private staging

## Status

Accepted for the host publication tool.

## Decision

The host publisher stages complete entries in a private `.incoming` directory inside the configured prepared root.
It supplies the approved digest-pinned OCI reference and Generation ID to `install_generation`.
The installer checks the staged entry's identity and fully verifies the manifest and referenced contents, including the OCI digest.
A same-filesystem move that refuses replacement publishes the entry under the reference hash.
Installers serialize changes to the root and refuse duplicate reference claims or substitution of an existing Generation.

The publisher owns the prepared root and its staging subtree, including the extraction and file permissions.
The runtime may read the published entry immediately, and a partially transferred entry remains outside its scan.
Runner admission also checks digest-pinned references against the certified OCI manifest during both local lookup and worker handoff.
Runtime discovery and pruning exclude the reserved staging directory.
Template version approval in MIOSA is a separate control-plane operation.

For a host which has the OCI layout locally, `scripts/publish-template-generation.sh` runs the release compiler, captures the boot snapshot, and invokes the atomic installer.
The caller supplies the approved OCI reference and memory size, as well as the verified release bundle, kernel, and filesystem tools.
The command emits the installed Generation ID on standard output only after verification and publication succeed.
The MIOSA caller must record that ID against the exact build and obtain host launch evidence before advertising the template on SOMA.
