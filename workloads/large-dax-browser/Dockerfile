# The SOMA large-shape DAX image with a live browser in it: `workloads/large-dax` plus Chrome for
# Testing, the shared libraries it links, and the fonts a page needs.
#
# Build the base first, then this image, both for the guest architecture:
#
#   docker build --platform=linux/amd64 -t soma-large-dax:3 workloads/large-dax
#   docker build --platform=linux/amd64 -t soma-large-dax-browser:1 workloads/large-dax-browser
#
# `BASE` is the base image by name. It is a build input rather than a `FROM` constant so the
# browser layer always sits on exactly the base a benchmark run is compared against.
#
# Ubuntu 24.04 ships `chromium` only as a snap stub, which cannot run in a guest with no snapd, so
# the browser is Chrome for Testing: a pinned build with a published digest, installed into
# /opt and linked as `chromium`, the name the live browser daemon looks for.
ARG BASE=soma-large-dax:3
FROM ${BASE}

ENV DEBIAN_FRONTEND=noninteractive

ARG CHROME_VERSION=155.0.8059.39
ARG CHROME_SHA256=55672d1f392fd3e7b7a08621b6e804e6bcb39d40cf155504abb74b3a021ea8ea

# The base trims apt to `noble noble-updates` and `main` on purpose, so the DAX workload's own
# `apt-get update` is cheap. Fonts live in `universe`, so it is switched on for this layer only
# and switched off again, with its index lists removed, before the layer ends. What is left on
# disk for `apt-get update` to compare against is exactly what the base left, plus nothing.
RUN cp /etc/apt/sources.list.d/ubuntu.sources /tmp/ubuntu.sources.base \
 && sed -i 's/^Components: main$/Components: main universe/' /etc/apt/sources.list.d/ubuntu.sources \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      fonts-liberation \
      fonts-noto-cjk \
      fonts-noto-color-emoji \
      fonts-noto-core \
      libasound2t64 \
      libatk-bridge2.0-0t64 \
      libatk1.0-0t64 \
      libcairo2 \
      libcups2t64 \
      libdbus-1-3 \
      libdrm2 \
      libgbm1 \
      libglib2.0-0t64 \
      libnspr4 \
      libnss3 \
      libpango-1.0-0 \
      libx11-6 \
      libxcb1 \
      libxcomposite1 \
      libxdamage1 \
      libxext6 \
      libxfixes3 \
      libxkbcommon0 \
      libxrandr2 \
 && fc-cache -f \
 && cp /tmp/ubuntu.sources.base /etc/apt/sources.list.d/ubuntu.sources \
 && rm -f /tmp/ubuntu.sources.base \
 && apt-get clean \
 && apt-get update \
 && apt-get clean

RUN curl -fsSL "https://storage.googleapis.com/chrome-for-testing-public/${CHROME_VERSION}/linux64/chrome-linux64.zip" \
      -o /tmp/chrome.zip \
 && echo "${CHROME_SHA256}  /tmp/chrome.zip" | sha256sum -c - \
 && unzip -q /tmp/chrome.zip -d /opt \
 && rm -f /tmp/chrome.zip \
 && ln -s /opt/chrome-linux64/chrome /usr/local/bin/chromium \
 && chromium --version

# The page-cache warm list the guest agent reads before a Generation is captured. Naming a list
# replaces the agent's conventional one, so `node` and `python3` are named too. The browser's
# libraries come from `ldd` at build time. See `workloads/browser-default` for the reasoning.
RUN mkdir -p /etc/soma \
 && { \
      echo /opt/chrome-linux64/chrome; \
      echo /opt/chrome-linux64/resources.pak; \
      echo /opt/chrome-linux64/icudtl.dat; \
      echo /opt/chrome-linux64/v8_context_snapshot.bin; \
      echo /usr/local/bin/node; \
      echo /usr/bin/python3; \
      ldd /opt/chrome-linux64/chrome | awk '/=> \//{print $3}'; \
    } | awk '!seen[$0]++' > /etc/soma/warm \
 && test "$(wc -l < /etc/soma/warm)" -gt 30
