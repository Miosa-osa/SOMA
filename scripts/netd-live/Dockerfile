# SOMA soma-netd live-test image.
# The base image is pinned by content digest so the iproute2, nftables, and
# conntrack userspace used by the live network tests come from one reviewed
# Ubuntu 24.04 image.  The test binaries are built on the host and bind-mounted.
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends \
      conntrack \
      iproute2 \
      iputils-ping \
      nftables \
 && apt-get clean

WORKDIR /work
