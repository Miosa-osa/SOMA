# SOMA guest-kernel builder image.
# The base image is pinned by content digest so the compiler, binutils, and make
# come from one reviewed Ubuntu 24.04 image.  build.sh records the resulting
# builder image identity and fails closed if the toolchain versions drift from
# toolchain.json.
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq --no-install-recommends \
      bc \
      binutils \
      bison \
      ca-certificates \
      flex \
      gcc \
      libc6-dev \
      libelf-dev \
      libssl-dev \
      make \
      perl \
      python3 \
      xz-utils \
 && apt-get clean

WORKDIR /work
