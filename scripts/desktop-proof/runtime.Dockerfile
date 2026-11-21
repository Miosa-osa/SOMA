# The room the desktop proof runs soma-netd and soma-api in.
#
# The proof is meant to run on a shared test host. soma-netd builds a network namespace, a veth
# pair, a TAP and nftables tables for each sandbox; inside this container all of that lives in the
# container's own network namespace, so the host's network configuration is never touched and
# `docker rm -f` takes the whole runtime away. The SOMA binaries are not in the image: they are
# bind-mounted from the build tree, so the proof always runs the checkout it was started from.
#
# The base is the same reviewed Ubuntu 24.04 the netd live tests pin (scripts/netd-live).
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends \
      ca-certificates \
      conntrack \
      curl \
      iproute2 \
      nftables \
      procps \
      python3 \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/*
