# SOMA XFS reflink measurement image.
# The base image is pinned by content digest so xfsprogs, e2fsprogs, and
# util-linux come from one reviewed Ubuntu 24.04 image.  The container runs
# privileged so it can attach loop devices and mount the XFS image that
# scripts/xfs-reflink-bench.sh creates on the host.  It never builds Rust code;
# the host builds the benchmark and test executables and mounts them read-only.
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends \
      coreutils \
      e2fsprogs \
      util-linux \
      xfsprogs \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /work
