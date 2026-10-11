# Docker in a SOMA guest

A SOMA guest has no init system and PID 1 is the guest agent, so Docker needs four things that systemd would otherwise give it.

| Need | Provided by |
| --- | --- |
| Engine, Compose and Buildx packages | `install-docker-ce.sh`, the same pinned script every MIOSA image uses |
| cgroup v2 with `cpu memory io pids cpuset` delegated | the agent, at early init, when `/usr/bin/dockerd` is in the image (`boot/docker.rs`) |
| A data root that is not on `OverlayFS` | the agent binds `<head>/docker` onto `/var/lib/docker` (`overlay2` refuses an overlay backing filesystem) |
| A way to start the engine | `soma-docker-ensure`, run by the `docker` wrapper on first use |
| The engine surviving the end of each command | `cgroup:/docker-engine` and `cgroup:/docker` lines in `/run/soma/survivors` (`survivors.rs`) |

The kernel needs the Docker facilities in `kernel/soma-v1.fragment` (netfilter, bridge, veth, cgroup device/blkio/BPF device filter, CFS bandwidth).
The writable head must be large enough for images and layers: `scripts/prepare-generation.sh` defaults to 10240 MiB.

## Limits

- The lazy start runs when the `docker` command on `PATH` runs.
  A client that talks to `/run/docker.sock` directly (an SDK) must call `soma-docker-ensure` first.
- The engine is not in the snapshot.
  The pre-capture sweep stops it and the first `docker` command after a restore starts it again, which costs the start time (measure it before claiming a number).
- Guest egress is required to pull images.
- Not proven: any of this on a booted SOMA guest.
  The agent logic is unit tested and the image stanza builds, but the kernel build, the Generation and the KVM run are the proof, and they are separate steps.
