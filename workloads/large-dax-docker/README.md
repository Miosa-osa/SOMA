# SOMA large-shape DAX image with Docker

`workloads/large-dax` plus Docker Engine, Compose and Buildx, started lazily by the `docker` command on `PATH`.
Mechanism and limits: `workloads/docker-engine/README.md`.

The image is a separate one on purpose.
The benchmark image stays byte-identical until a before and after run says Docker costs nothing.
Docker is not in the warm list, so it adds no page cache to the snapshot, and the install script removes the Docker apt source so `apt-get update` does the same two fetches as before.
What changes is the root filesystem size (about 600 MB more) and nothing at run time until the first `docker` command.

Compile it with the large machine shape, the large kernel and a network device:

```sh
SOMA_PREPARE_VCPUS=8 SOMA_PREPARE_NETWORK=public_internet \
  target/release/examples/prepare_generation <reference> <oci-layout> \
  kernel/out-large/vmlinux-6.12.107-soma-large kernel/out-large/final.config \
  target/x86_64-unknown-linux-musl/release/soma-guest-agent \
  <fs-tools>/erofs <fs-tools>/e2fsprogs <out-entry> 16384 20480
```
