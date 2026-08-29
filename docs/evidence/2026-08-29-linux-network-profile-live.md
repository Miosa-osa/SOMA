# Linux network profile live proof - 2026-08-29

## Evidence boundary

This result proves that `soma-netd`, running as a privileged process inside the pinned Ubuntu 24.04 container on the real host kernel, can prepare a sterile network bundle whose guest link is down and whose namespace forwards nothing, atomically assign that bundle to an Instance while producing the exact `LaunchNetwork` values, hand its TAP descriptor to a receiver over `SOCK_SEQPACKET` with `SCM_RIGHTS` and a fixed typed header, keep the guest silent until the caller attests authenticated repair, activate forwarding so that the gateway, a public listener, and the declared resolver become reachable through masquerade, drop the cloud metadata address, an undeclared resolver, the host address, a peer guest, and the peer's gateway under the same `PublicInternet` egress, release both bundles completely, reconcile the ledger against the kernel with no unowned object, and run one hundred prepare, assign, activate, release cycles that leave no namespace pin, link, or table behind.

It does not prove a jailed VMM, a virtio-net attach of the transferred TAP, traffic from a guest Linux kernel, IPv6 guest traffic, ingress forwarding, proxy attachment, the daemon socket end to end, crash recovery from a partially torn down bundle, broker behavior with `CAP_NET_ADMIN` outside a container, or any latency objective.
The timings are diagnostic numbers from a debug build inside a container on a busy development host and are not a benchmark.

## Identities

- SOMA Git revision of the retained run: `77b8ec0` on `feat/soma-netd`, whose base was `abc7034`; the branch was rebased onto `origin/main` `9f3a656` after the run, and that rebase changed documentation files only.
- Host: Ubuntu 24.04.4 LTS, `Linux 7.0.0-30-generic #30~24.04.1-Ubuntu SMP PREEMPT_DYNAMIC` x86_64, Intel Core Ultra 9 275HX, Docker 29.3.0.
  The host user holds no `CAP_NET_ADMIN`, so every kernel operation ran inside the container, which shares the host kernel.
- Container: `docker run --rm --privileged` of `soma-netd-live:local`, image id `sha256:3b700443b25f122b407d8d43ce816569ab8c7cd1a2a85f6c1c9369cf3b991041`, built by `scripts/netd-live-tests.sh` from `scripts/netd-live/Dockerfile` on the base `ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517`, the same pinned base as `kernel/Dockerfile`.
  The container reported `iproute2-6.1.0`, `nftables v1.0.9`, and `conntrack v1.4.8`; the broker executes only `/usr/sbin/nft` and `/usr/sbin/conntrack`, while `ip` is used by the test's world fixture alone.
- Rust toolchain `1.98.0 (88d9e12ae 2026-08-18)`; the `live_linux` test executable was built on the host by `cargo test --locked -p soma-netd --test live_linux --no-run` in the debug profile and bind-mounted read-only into the container.
- Broker profile under test: uplink `uplink0`, guest lease plan `10.200.0.0/16` carved into `/30` leases, transit plan `10.201.0.0/16` carved into `/30` leases, declared resolver `1.1.1.1`, host address `203.0.113.1`, cleanup generation `1`, and a ledger in a fresh temporary directory on the container's overlay filesystem.
- World stand-in: namespace `soma-world` behind `uplink0` answering on `203.0.113.10:8080` over TCP, on `169.254.169.254:80` over TCP as the metadata stand-in, and on `1.1.1.1:53` and `8.8.8.8:53` over UDP with the reply `dns-ok`; the host side routes those four addresses through `203.0.113.10`, so every silence below is a policy decision rather than a missing route.

## Invocation

```sh
SOMA_NETD_LIVE_LOG=live-run3.log scripts/netd-live-tests.sh
```

Inside the container the script runs `/work/live_linux --ignored --test-threads=1 --nocapture` and then prints `ip netns list`, `ip -brief link`, and `nft list tables`.
Both tests carry `#[ignore]` and call `require_privilege()`, which panics with `prerequisite failed: ...` when a namespace cannot be created, `/dev/net/tun` is absent, or `/usr/sbin/nft` is absent, so an unprivileged `cargo test --workspace` reports them as `2 ignored` and never as passed.

## What passed

`sterile_bundle_stays_down_until_activation_and_policy_holds_after_it` observed, in order:

| Step | Observation |
|---|---|
| Sterile bundle | inside the sandbox namespace `net.ipv4.ip_forward` was not `1` and `tap0` did not carry the `UP` flag |
| Assign | `LaunchNetwork` was exactly address `10.200.0.2` with prefix `30`, gateway `10.200.0.1`, resolver `1.1.1.1`, vsock CID `5`, generation `1`, and the guest MAC derived from the `BundleId` |
| TAP transfer | `send_tap` over a `SOCK_SEQPACKET` pair delivered one descriptor and `receive_tap` returned a header equal to the sent bundle, generation, and intent digest |
| Before activation | the guest stand-in's ARP request for `10.200.0.1` got no reply within 700 ms and a TCP SYN to `203.0.113.10:8080` got silence |
| Activate | `ActivationEvidence` reported `forwarding: true` and three raised links, `tap0`, `vs0`, and the host veth; the gateway then answered ARP |
| Gateway | ICMP echo to `10.200.0.1` was answered |
| Public egress | a TCP SYN to `203.0.113.10:8080` was answered with SYN-ACK and the world listener recorded the peer as `203.0.113.1`, the masqueraded host address |
| Metadata | a TCP SYN to `169.254.169.254:80` got silence while a host-side `connect` to the same listener succeeded |
| Declared DNS | UDP to `1.1.1.1:53` returned `dns-ok` |
| Undeclared DNS | UDP to `8.8.8.8:53` got silence |
| Host address | ICMP echo to `203.0.113.1` got no reply |
| Second bundle | bundle B at `10.200.0.6` with gateway `10.200.0.5` and CID `6` activated and reached `203.0.113.10:8080`; guest A's SYN to `10.200.0.6:8080`, ping to `10.200.0.6`, and ping to `10.200.0.5` all got silence, and the two conntrack zones differ |
| Release A | `complete` and `ledger` were true; reconcile reported two entries, A `Released` and B `Consistent`, with zero unowned objects |
| Release B | `complete` was true, the namespace pin directory was empty, and reconcile reported every entry `Released` with zero unowned objects |

`hundred_way_prepare_assign_activate_release_burst` prepared one hundred distinct bundles, assigned them with vsock CIDs 3 through 102, activated, and released each in sequence; every `ReleaseEvidence.complete` was true, reconcile reported one hundred `Released` entries and zero unowned namespaces, links, or tables, and the pin directory was empty.

The script's post-run listing of the container was:

```text
==> post-run namespaces (expect none):
==> post-run links (expect only lo and the container uplink):
lo               UNKNOWN        00:00:00:00:00:00 <LOOPBACK,UP,LOWER_UP>
eth0@if140       UP             36:1c:b9:76:46:77 <BROADCAST,MULTICAST,UP,LOWER_UP>
==> post-run nft tables (expect none):
```

The test harness summary was `test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.09s`.

## Timings

Per-operation wall time of the 100-way burst in the retained run, measured with the monotonic clock around each library call in a debug build on one thread.
With one hundred samples the p99 is the largest sample.

| Operation | min | p50 | p99 | max |
|---|---:|---:|---:|---:|
| prepare | 16,966,125 ns (17.0 ms) | 30,023,566 ns (30.0 ms) | 46,422,874 ns (46.4 ms) | 46,422,874 ns |
| assign | 9,070,535 ns (9.1 ms) | 15,816,521 ns (15.8 ms) | 23,184,316 ns (23.2 ms) | 23,184,316 ns |
| activate | 3,112,528 ns (3.1 ms) | 3,509,118 ns (3.5 ms) | 4,717,014 ns (4.7 ms) | 4,717,014 ns |
| release | 33,198,400 ns (33.2 ms) | 55,343,344 ns (55.3 ms) | 77,117,970 ns (77.1 ms) | 77,117,970 ns |

Two earlier runs of the same tests on the same host, before the world fixture deleted its uplink synchronously, gave p50 and p99 of prepare 31.1 and 44.3 ms then 30.4 and 43.7 ms, assign 21.1 and 31.1 ms then 20.2 and 30.3 ms, activate 4.1 and 5.8 ms then 3.8 and 5.1 ms, and release 53.5 and 91.0 ms then 52.0 and 71.1 ms, so the retained numbers are representative rather than a best case.

`prepare` spawns `nft` twice, and `release` spawns `conntrack` once and `nft` up to three times, so the version 1 subprocess mechanism dominates both; `activate` performs only `ioctl` calls and `/proc/sys` writes.
Replacing the subprocess seam with a netlink and libnftnl binding is the identified lever, not a kernel limit.

Raw samples of the retained run in nanoseconds, one line per operation:

```text
burst raw op=prepare ns=[16966125, 20506065, 20693560, 20866028, 20985669, 21203127, 21311267, 21684362, 22039896, 22146833, 22382645, 23121548, 24077476, 24394728, 24526177, 24616913, 24769372, 25519498, 25738583, 26338304, 26380484, 26582655, 26606337, 26803013, 26827574, 26966692, 27188549, 27422104, 27507628, 27526770, 27587098, 27843456, 27897333, 28201133, 28382058, 28609546, 28885114, 29043872, 29077211, 29132245, 29178502, 29192931, 29204232, 29249042, 29314476, 29690999, 29731244, 29802211, 29865404, 29884740, 30023566, 30036320, 30097863, 30179218, 30246439, 30366782, 30466367, 30466770, 30532414, 31073506, 31473806, 31510149, 31529914, 31640024, 31753840, 31828852, 31942906, 32110997, 32211632, 32386966, 32468234, 32562546, 32823754, 32989608, 33052557, 33165679, 33250876, 33531822, 33832845, 33864259, 34320625, 34555257, 34959297, 34995385, 35694505, 35773997, 35796086, 35924794, 37795653, 38101327, 38367915, 38403816, 38862224, 39236920, 39661026, 40298965, 40405101, 41266926, 42189965, 46422874]
burst raw op=assign ns=[9070535, 10016703, 10989154, 10999166, 11041798, 11082243, 11162556, 11771806, 11892246, 11922694, 11970116, 11986601, 12002835, 12053218, 12084454, 12085358, 12093509, 12116243, 12811890, 12875692, 12978920, 12981893, 13011829, 13017035, 13032678, 13062145, 13124125, 13184406, 13186253, 13838491, 13902282, 13923013, 13947083, 13948368, 13956752, 13978767, 13984018, 13985604, 14008365, 14021295, 14076239, 14099826, 14158462, 14212920, 14927637, 15015035, 15016308, 15174658, 15221773, 15765499, 15816521, 15838442, 15855105, 15898940, 15959406, 15971099, 15985159, 16016747, 16058112, 16061323, 16083003, 16089466, 16113010, 16114731, 16125321, 16138373, 16175780, 16293104, 16876988, 16883085, 16883352, 16936379, 16940476, 17031137, 17062246, 17097242, 17108820, 17114958, 17857189, 17930168, 17950658, 17956253, 17983507, 17985485, 18068379, 19020169, 19031502, 19045145, 19208504, 19909125, 19976695, 20080250, 20081295, 20086096, 20846874, 20912785, 22065923, 22966758, 23004462, 23184316]
burst raw op=activate ns=[3112528, 3135988, 3148533, 3169562, 3173749, 3183443, 3192284, 3210171, 3218325, 3224279, 3241802, 3246773, 3246821, 3252546, 3255070, 3274006, 3277421, 3280798, 3294069, 3310091, 3335601, 3338696, 3341179, 3347901, 3353439, 3360123, 3364737, 3365517, 3373175, 3374738, 3383172, 3397769, 3426312, 3429072, 3435736, 3440233, 3441101, 3443173, 3445394, 3446299, 3447791, 3452508, 3461271, 3471009, 3472178, 3476238, 3489928, 3492364, 3497738, 3506928, 3509118, 3509130, 3514831, 3515933, 3518517, 3519313, 3519572, 3521267, 3529326, 3533375, 3536335, 3536972, 3545269, 3562111, 3568058, 3577722, 3581617, 3600000, 3602187, 3608782, 3613628, 3635117, 3656447, 3676580, 3680008, 3707192, 3711565, 3717175, 3731698, 3734149, 3753578, 3760975, 3760993, 3788164, 3794909, 3800760, 3823137, 3835217, 3911038, 3929091, 3964566, 3973247, 3986825, 4128449, 4185961, 4192058, 4298843, 4385214, 4398053, 4717014]
burst raw op=release ns=[33198400, 33662821, 34253155, 34300988, 35196210, 35652673, 36257865, 36293176, 37671288, 38194237, 38202442, 38395084, 39587030, 39799963, 39800113, 39906557, 40123832, 40373907, 41726982, 41786852, 41808337, 42994150, 43400798, 43687426, 43696052, 43935698, 44115912, 44555592, 44671781, 44881991, 45425551, 45624428, 45824480, 45937525, 46161272, 46316127, 46368951, 46488127, 46926395, 47342010, 47758942, 48041512, 49043812, 49063187, 49300348, 49527104, 49976631, 50760085, 52028137, 55006308, 55343344, 56404676, 56930998, 57481445, 57754974, 57887489, 58889478, 59342841, 59701253, 59815589, 60242223, 60326066, 60416758, 60707748, 60767310, 60863450, 61055860, 61257706, 61329064, 61491669, 61501498, 61563188, 61679240, 61981921, 62026567, 62162617, 62376823, 62519469, 62588837, 62653556, 62682716, 62782860, 63299297, 63358491, 63417858, 63505733, 63556032, 64531068, 64788608, 64894465, 64961353, 65537566, 65709651, 66170071, 66361873, 66630817, 66903708, 67563166, 69833570, 77117970]
```

## Not exercised

- The `Unrestricted` and `Denied` egress classes did not run live.
  In version 1 the rendered sandbox ruleset for `Unrestricted` is identical to `PublicInternet`, because both add the same single forward accept after the protected drops, and `Denied` adds none; `crates/soma-netd/src/firewall/tests.rs` asserts on the rendered text that every protected prefix is dropped before any accept in all three classes.
- IPv6 guest traffic: the broker disables IPv6 inside the sandbox namespace and both tables drop every IPv6 frame from the guest, but no IPv6 probe was sent.
- Source spoofing: both tables drop frames whose Ethernet source is not the assigned MAC or whose IPv4 source is not the lease; the guest stand-in sends only its own addresses, so those rules are proven at the text level only.
- `DnsPolicy::Custom`, `DnsPolicy::Denied`, ingress publication after Ready, port races, and proxy attachment were not exercised live; ingress ports are reserved but never forwarded, and proxy attachment is typed `Unimplemented`.
- The daemon binary was not driven over its Unix socket; its frame codec is unit tested and the library calls it composes are the ones proven here.
- Crash recovery through `release_record` over a ledger entry whose owning process died, and reconcile over a partially torn down bundle, were not exercised live; reconcile ran only over consistent and released entries.
- Nothing ran with `CAP_NET_ADMIN` on the bare host, because this host grants none; the container shares the host kernel, so the kernel behavior is real, but the root namespace is the container's.
- The ledger lived on the container's overlay filesystem; the create-exclusive, sync, and hard-link sequence ran, which is not a proof of durability across host power loss.
- The transferred TAP was driven by a test stand-in through `read` and `write`; no VMM attached it to a virtio-net device and no guest kernel sent a frame.
