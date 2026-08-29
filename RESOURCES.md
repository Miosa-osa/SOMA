# SOMA resources

This file records the primary sources that constrain SOMA's design.

## Kernel and virtualization interfaces

- [Linux KVM API](https://www.kernel.org/doc/html/latest/virt/kvm/api.html) defines the file-descriptor, ioctl, memory-mapping, and vCPU execution contract.
- [rust-vmm community](https://github.com/rust-vmm/community) catalogs reusable Rust virtualization crates and production consumers.
- [rust-vmm kvm-ioctls](https://github.com/rust-vmm/kvm) provides Rust bindings and wrappers for KVM.
- [rust-vmm vm-superio](https://github.com/rust-vmm/vm-superio) provides the bounded 16550 UART model used by the ARM64 command proof.
- [Apple Containerization ARM64 kernel configuration](https://github.com/apple/containerization/blob/2faaf9b4aff48a4745ef3d26c3f1450c1228fdf0/kernel/config-arm64) pins the reviewed nested-development kernel inputs and exposes its 8250 device-count limits.

## Networking and host-isolation interfaces

- [Firecracker network setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/network-setup.md) documents its TAP backend, host routing choices, nftables setup, ingress, egress, and multi-guest considerations.
- [Firecracker production host setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/prod-host-setup.md) states that Firecracker does not filter guest traffic and documents host-side metadata filtering and network-flood mitigations.
- [Firecracker jailer](https://github.com/firecracker-microvm/firecracker/blob/main/docs/jailer.md) documents jailed device access and the requirements for multiple TAP interfaces.
- [Linux TUN/TAP documentation](https://docs.kernel.org/networking/tuntap.html) defines TAP allocation, file-descriptor ownership, Ethernet-frame I/O, and multiqueue behavior.
- [Linux network namespaces](https://man7.org/linux/man-pages/man7/network_namespaces.7.html) define isolation for network devices, routes, firewall rules, sockets, and veth lifecycle.
- [Linux veth](https://man7.org/linux/man-pages/man4/veth.4.html) defines the paired virtual-Ethernet mechanism used to connect network namespaces.
- [nftables manual](https://netfilter.org/projects/nftables/manpage.html) defines packet-filtering hooks, NAT, named sets, maps, and conntrack zones.
- [Linux `IPV6_V6ONLY`](https://man7.org/linux/man-pages/man2/IPV6_V6ONLY.2const.html) defines whether an IPv6 socket also accepts IPv4-mapped traffic and explains why SOMA records this setting explicitly.
- [Linux Unix-domain sockets](https://man7.org/linux/man-pages/man7/unix.7.html) define `SOCK_SEQPACKET`, peer credentials, `SCM_RIGHTS`, and file-descriptor transfer for the privileged broker seam.
- [Apple Container command reference](https://github.com/apple/container/blob/main/docs/command-reference.md) defines named networks, `--network`, `--dns`, `--no-dns`, and host-port publication for the macOS development adapter.
- [Apple Container port and network parser](https://github.com/apple/container/blob/main/Sources/Services/ContainerAPIService/Client/Parser.swift) is the primary implementation reference for accepted publication syntax, TCP and UDP selection, address parsing, and current port limits.
- [AWS Instance Metadata Service](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/configuring-instance-metadata-service.html) defines the AWS metadata IPv4 and IPv6 endpoints that certified AWS profiles must protect.
- [Google Compute Engine metadata](https://cloud.google.com/compute/docs/metadata/overview) defines the Google metadata DNS name and IPv4 and IPv6 endpoints that certified Google Cloud profiles must protect.
- [Azure Instance Metadata Service](https://learn.microsoft.com/en-us/azure/virtual-machines/instance-metadata-service) defines the Azure host-local metadata endpoint that certified Azure profiles must protect.
- [CNI specification](https://www.cni.dev/docs/spec/) is a reference for future network-runtime adapters, lifecycle idempotency, runtime capabilities, and result validation rather than SOMA's portable public interface.

## Production VMM references

- [Firecracker design](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) documents its one-process-per-microVM thread model and minimal device surface.
- [Firecracker snapshot memory implementation](https://github.com/firecracker-microvm/firecracker/blob/main/src/vmm/src/vstate/memory.rs) is a reference for private snapshot-backed guest memory.
- [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) is a production Rust VMM built from rust-vmm crates.
- [crosvm architecture](https://chromium.googlesource.com/crosvm/crosvm/+/main/ARCHITECTURE.md) is a reference for device isolation and process topology tradeoffs.
- [OpenVMM](https://github.com/microsoft/openvmm) is a modular cross-platform Rust VMM.
- [StratoVirt](https://gitee.com/openeuler/stratovirt) is a Rust KVM VMM supporting microVM and standard machine profiles.
- [Dragonball in Kata Containers](https://github.com/kata-containers/kata-containers/blob/main/docs/design/virtualization.md) is a Rust VMM integrated into a container runtime.
- [Apple container technical overview](https://github.com/apple/container/blob/main/docs/technical-overview.md) documents its Virtualization.framework VM-per-container architecture and supporting services.

## Sandbox and restore references

- [ComputeSDK benchmarks](https://github.com/computesdk/benchmarks) define the public create-through-first-command benchmark SOMA must reproduce exactly.
- [OCI image specification](https://github.com/opencontainers/image-spec) defines immutable manifests, indexes, layers, configuration, and platform selection for input images.
- [OCI distribution specification](https://github.com/opencontainers/distribution-spec) defines registry distribution and digest-addressed content behavior.
- [CubeSandbox](https://github.com/TencentCloud/CubeSandbox) documents Tencent Cloud's CubeHypervisor, CubeShim, Cubelet, networking, and sandbox topology.
- [Kuasar](https://github.com/kuasar-io/kuasar) separates a multi-sandbox runtime from the VMMs it launches.
- [Quark](https://github.com/QuarkContainer/Quark) pairs its QVisor VMM with a specialized QKernel guest.
- [Mitos](https://github.com/mitos-run/mitos) demonstrates prepared and preclaimed Firecracker restore paths and publishes measurement details.
- [SporeVM](https://github.com/sporevm/sporevm) is a Zig VMM research reference for snapshot fan-out and explicit readiness gates.
- [Machinen](https://github.com/redwoodjs/machinen) is a Zig VMM reference with published benchmark artifacts.
- [smolvm](https://github.com/smol-machines/smolvm) demonstrates live copy-on-write fork mechanisms and their security tradeoffs.
- [Vibemon](https://github.com/can1357/vibemon) is a custom Rust VMM and sandbox research implementation.
- [Zeroboot pinned KVM restorer](https://github.com/zerobootdev/zeroboot/blob/87ca9c018a9c2a343ece768eec508e16497753f9/src/vmm/kvm.rs) is a compact research reference for raw KVM snapshot restoration with private memory mapping.
- [The State of MicroVM Isolation in 2026](https://emirb.github.io/blog/microvm-2026/) is a first-person ecosystem survey of VMM tradeoffs, rust-vmm reuse, snapshot latency, and VM-backed agent sandboxes.

## Deployment substrates

- [AWS EC2 nested virtualization](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/amazon-ec2-nested-virtualization.html) documents supported nested KVM instance types and recommends bare metal for latency-sensitive virtualization workloads.
- [AWS Nitro bare-metal instances](https://docs.aws.amazon.com/ec2/latest/instancetypes/ec2-nitro-instances.html) document host-hardware access for workloads requiring low-level virtualization features.
- [Google Compute Engine nested virtualization](https://cloud.google.com/compute/docs/instances/nested-virtualization/creating-nested-vms) documents KVM-backed nested guest setup.

## Fleet control-plane references

- [AWS cell-based architecture guidance](https://docs.aws.amazon.com/wellarchitected/latest/reducing-scope-of-impact-with-cell-based-architecture/why-to-use-a-cell-based-architecture.html) explains fixed-size independently testable cells as units of scale and failure containment.
- [Google's Borg paper](https://research.google/pubs/large-scale-cluster-management-at-google-with-borg/) documents admission, task placement, machine sharing, correlated-failure reduction, and operation across clusters with tens of thousands of machines.
- [Kubernetes large-cluster guidance](https://kubernetes.io/docs/setup/best-practices/cluster-large/) documents explicit single-cluster scale envelopes and the cloud quotas that become part of capacity planning.
- [HashiCorp Nomad's architecture overview](https://developer.hashicorp.com/nomad/docs/what-is-nomad) is a reference for multi-region federation and independently published million-container stress exercises.
- [Making retries safe with idempotent APIs](https://aws.amazon.com/builders-library/making-retries-safe-with-idempotent-APIs/) explains caller-provided operation identity, semantic replay, changed-intent conflicts, and late-arrival handling for distributed mutations.

## Linux memory and storage mechanisms

- [Linux mmap API](https://www.man7.org/linux/man-pages/man2/mmap.2.html) defines private mappings, no-reserve behavior, and their failure semantics.
- [Linux userfaultfd documentation](https://docs.kernel.org/admin-guide/mm/userfaultfd.html) defines userspace-managed page faults and their scheduling and registration costs.
- [Linux FICLONE documentation](https://man7.org/linux/man-pages/man2/FICLONE.2const.html) defines shared-extent copy-on-write semantics without promising a constant latency distribution.

## Security references

- [Dragonball virtio-blk advisory](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-fgm4-mv68-h344) demonstrates why guest-controlled lengths require checked bounds before host I/O.
- [Erlang NIF documentation](https://www.erlang.org/docs/26/man/erl_nif.html) explains why unsafe VMM state must not live inside the BEAM process.
- [seccomp userspace API](https://www.kernel.org/doc/html/latest/userspace-api/seccomp_filter.html) defines the Linux syscall filtering mechanism.

## Licensing

- SOMA is licensed under [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
- Vendored or derived code must retain its original provenance, copyright, license, and NOTICE obligations.

## Documentation references

- [Modal's GPU Glossary](https://modal.com/gpu-glossary/readme) is a documentation-pattern reference for an interlinked glossary that connects terms across multiple technical layers.
- SOMA glossary definitions remain original to this project and use the accepted terminology in `docs/architecture/naming.md` and the architecture decision records.
