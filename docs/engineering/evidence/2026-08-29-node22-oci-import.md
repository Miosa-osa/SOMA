# Node 22 OCI import verification - 2026-08-29

## Evidence boundary

This result proves that SOMA revision `f9a7e1be615d4681c72136912fb7daf117e24d8f` can import two extracted OCI layouts for the same real `node:22` ARM64 image, verify every selected descriptor and layer, and derive the same deterministic import identity despite different top-level archive bytes.
It does not prove root filesystem construction, a bootable Generation, sandbox creation, guest readiness, snapshot restore, network isolation, cleanup, or launch latency.

## Identities

- SOMA Git revision: `f9a7e1be615d4681c72136912fb7daf117e24d8f`.
- Host: Apple M3 Ultra, macOS 26.5 build 25F71, ARM64.
- Rust compiler: `rustc 1.98.0 (88d9e12ae 2026-08-18)`.
- Cargo: `cargo 1.98.0 (797e8a9bc 2026-08-05)`.
- Selected OCI platform: `linux/arm64/v8`.
- Selected OCI manifest: `sha256:2f22d3b5ec6552b890773a152030b1360d35da0c4369799319523ccdb2d78e0e`.
- Selected OCI config: `sha256:77e2f8d0fcba638dd3b6d10383fc7cb59f3e8198d43b25380c103dfb73cf22e5`.
- Derived import manifest: `sha256:7f054135dc1553375fb1e798b902f5580c745741d45c4d6f3088e08bbaac110e`.

## Inputs

The first OCI archive was 399,676,928 bytes with SHA-256 `62b2fe24e2a98ddf8a798f3d5e7cb48f93435653c727faf9e13eadd2222d204f`.
The second OCI archive was 399,676,928 bytes with SHA-256 `78bfc1b4a93f5a22ac3268c34792cbaf169be9b0961f96cc12719522ffd62e61`.
Their top-level `index.json` files also differed, with SHA-256 values `11717c6b38ab1cad776b01327e2869d8977e038f0e598820fd0a5fc8638f2237` and `08d8ed9afdd5cc71675e81349874142d42a83c5bb9bd1f75641d43ed134cf0ed`.
Both layouts selected the same nested image manifest, config, platform, and eight image layers.

## Invocation

The ignored live integration test was selected explicitly with two extracted layout directories.

```sh
SOMA_OCI_LAYOUT=/path/to/first/layout \
SOMA_OCI_LAYOUT_SECOND=/path/to/second/layout \
cargo test --locked -p soma-generation \
  --test live_apple_layout -- --ignored --exact \
  imports_an_extracted_real_apple_container_layout --nocapture
```

## Results

| Import | Result | Verification time | Import manifest | Stored blobs | Stored bytes | Traversed indexes |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| First extracted layout | Pass | 28.101141541 s | `sha256:7f054135dc1553375fb1e798b902f5580c745741d45c4d6f3088e08bbaac110e` | 13 | 399,651,890 | 2 |
| Second extracted layout | Pass | 27.908527750 s | `sha256:7f054135dc1553375fb1e798b902f5580c745741d45c4d6f3088e08bbaac110e` | 13 | 399,651,890 | 2 |

The canonical import manifest size was 3,021 bytes for both layouts.
The test process completed with one passing test and no failures in 56.05 seconds.

## Measurement warning

These times measure cold offline validation of already extracted OCI layouts on the development Mac.
They include descriptor reads, hashing, gzip expansion, tar structure validation, layer diff-ID verification, and content-addressed publication.
They exclude registry transfer, archive export, archive extraction, root filesystem application, kernel boot, guest readiness, command execution, and cleanup.
They are Generation-input verification measurements and must not be presented as sandbox launch latency.
