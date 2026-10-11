# The SOMA large-shape DAX image with Docker in it: `workloads/large-dax` (or the browser variant,
# through BASE) plus Docker Engine, Compose and Buildx, started lazily.
#
# Build the base first, then this image, both for the guest architecture, with the engine
# directory as a named build context:
#
#   docker build --platform=linux/amd64 -t soma-large-dax:3 workloads/large-dax
#   docker build --platform=linux/amd64 --build-context docker-engine=workloads/docker-engine \
#     -t soma-large-dax-docker:1 workloads/large-dax-docker
#
# `BASE` is a build input, as in `workloads/large-dax-browser`, so Docker sits on exactly the base
# a benchmark run is compared against. Pass `--build-arg BASE=soma-large-dax-browser:1` for the
# image that also carries the browser.
#
# What it must not change: the base keeps `apt-get update` to two `Hit` lines. The install script
# removes the Docker apt source and its index lists when it is done, so the workload's own update
# fetches nothing it did not fetch before. The package list the workload installs is untouched.
ARG BASE=soma-large-dax:3
FROM ${BASE}

ENV DEBIAN_FRONTEND=noninteractive

# The guest has no init system, so nothing starts the engine at boot and the snapshot holds no
# daemon. `/usr/local/bin/docker` runs `soma-docker-ensure` first, which starts dockerd on first
# use (see workloads/docker-engine/README.md). The pinned install script expects systemd, so a
# no-op systemctl stands in for the duration of the install only.
COPY --from=docker-engine install-docker-ce.sh /tmp/install-docker-ce.sh
RUN printf '#!/bin/sh\nexit 0\n' > /usr/bin/systemctl \
 && chmod 0755 /usr/bin/systemctl \
 && printf '#!/bin/sh\nexit 101\n' > /usr/sbin/policy-rc.d \
 && chmod 0755 /usr/sbin/policy-rc.d \
 && DOCKER_SOCKET_ACTIVATED=0 bash /tmp/install-docker-ce.sh \
 && rm -f /usr/bin/systemctl /usr/sbin/policy-rc.d /tmp/install-docker-ce.sh \
 && test -x /usr/bin/dockerd \
 && /usr/bin/docker --version && /usr/bin/docker compose version && /usr/bin/docker buildx version
COPY --from=docker-engine --chmod=0644 daemon.json /etc/docker/daemon.json
COPY --from=docker-engine --chmod=0755 soma-docker-ensure /usr/local/libexec/soma-docker-ensure
COPY --from=docker-engine --chmod=0755 docker-wrapper /usr/local/bin/docker
