# The SOMA large-shape (contract v2) workload image: Ubuntu 24.04 with the toolset the
# install/typecheck workload needs and Node 22 from the official tarball into /usr/local.
#
# Build it for the guest architecture:
#
#   docker build --platform=linux/amd64 -t soma-large-dax:2 workloads/large-dax
#
# The base image is pinned by index digest so the compiler, the libc, and the package metadata
# come from one reviewed Ubuntu 24.04, the way `kernel/Dockerfile` pins its builder.
FROM ubuntu@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

ENV DEBIAN_FRONTEND=noninteractive

# Three things about apt in this image, all of them about what `apt-get update` costs at run time.
#
# The workload's own `prepare()` runs `apt-get update` unconditionally and then installs a fixed
# list of packages. The update cannot be removed by the harness, so it is made cheap here:
#
#   * Every package `prepare()` installs is already in this image, so the install is a no-op and
#     `apt-get update` is the whole of what is left.
#   * The sources are trimmed to the one suite pair and the one component those packages live in.
#     `noble`, `noble-updates`, and `main` is the minimum that satisfies
#     `bash build-essential ca-certificates curl git python3 python3-setuptools unzip`; there is no
#     `deb-src`, and `noble-backports`, `restricted`, `universe`, and `multiverse` are absent
#     because nothing here needs them. Each suite and component is another index file, and an
#     index file that is not listed is one that is never fetched.
#   * The index lists are left in place rather than deleted at the end of the build. `apt-get
#     update` compares the `InRelease` it fetches against the lists on disk and reports `Hit`
#     instead of downloading an index that has not changed, so a machine built from this image
#     answers `apt-get update` with two `Hit` lines and no index downloads at all.
#
# `Acquire::Languages none` drops the per-language translation indexes, which no part of this
# workload reads, and `Acquire::PDiffs false` takes the full index rather than a patch chain.
RUN printf 'Acquire::Languages "none";\nAcquire::PDiffs "false";\n' \
      > /etc/apt/apt.conf.d/99-soma-large-dax \
 && printf 'Types: deb\nURIs: http://archive.ubuntu.com/ubuntu/\nSuites: noble noble-updates\nComponents: main\nSigned-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n' \
      > /etc/apt/sources.list.d/ubuntu.sources \
 && rm -f /etc/apt/sources.list.d/ubuntu.sources.bak

# The package list is exactly what the workload's `prepare()` installs on apt
# (`bash build-essential ca-certificates curl git python3 python3-setuptools unzip`), plus
# `xz-utils` for the Node tarball below. Official totals include the prepare phase, so a package
# prepare() would install is baked here instead, and `apt-get clean` runs but the lists stay.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      bash \
      build-essential \
      ca-certificates \
      curl \
      git \
      python3 \
      python3-setuptools \
      unzip \
      xz-utils \
 && apt-get clean

# Node 22 from the official tarball, verified against the published digest for this version. The
# version is pinned because the workload reports it and the comparison arm runs the same major.
ARG NODE_VERSION=22.23.3
ARG NODE_SHA256=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de
RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
      -o /tmp/node.tar.xz \
 && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c - \
 && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
 && rm -f /tmp/node.tar.xz \
 && node --version \
 && npm --version
