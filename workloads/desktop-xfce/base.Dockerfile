# A public desktop base for the SOMA desktop stage (Dockerfile in this directory).
#
# It carries what the stage expects of any desktop base, built from public sources only:
# Ubuntu 24.04, Xfce 4 with its terminal, D-Bus with the X11 helpers, the X utilities the session
# waits with, and KasmVNC 1.4.0 as X server and viewer, configured the way MIOSA's Firecracker
# desktop configures it (plain WebSocket on 6080, TLS and authentication left to the platform).
#
#   docker build -f workloads/desktop-xfce/base.Dockerfile -t soma-desktop-base:1 workloads/desktop-xfce
#
# Pins: the base by index digest (the same reviewed ubuntu:24.04 the kernel builder and the netd
# live tests use), and the KasmVNC package by the SHA-256 GitHub publishes for the release asset.
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl dbus dbus-x11 fonts-dejavu-core procps sudo xauth \
      xfce4 xfce4-terminal x11-utils x11-xserver-utils \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete

ARG KASMVNC_VERSION=1.4.0
ARG KASMVNC_SHA256=12bac6014149c5fdee75f0d403785aaa3e5dd4ea222de73253a5d4181bc9567e
RUN set -e; \
    deb="kasmvncserver_noble_${KASMVNC_VERSION}_amd64.deb"; \
    curl -fsSL --retry 3 -o "/tmp/${deb}" \
      "https://github.com/kasmtech/KasmVNC/releases/download/v${KASMVNC_VERSION}/${deb}"; \
    echo "${KASMVNC_SHA256}  /tmp/${deb}" | sha256sum -c -; \
    apt-get update; \
    apt-get install -y --no-install-recommends "/tmp/${deb}"; \
    rm -f "/tmp/${deb}"; \
    apt-get clean; \
    find /var/lib/apt/lists -type f -delete; \
    command -v Xkasmvnc

# KasmVNC's packaged defaults point at the snakeoil certificate, whose key the desktop user cannot
# read, which turns 6080 into a TLS port that closes every connection. The platform terminates TLS
# itself, so the certificate paths are cleared and 6080 stays plain WebSocket.
RUN mkdir -p /etc/kasmvnc \
 && printf '%s\n' \
      'network:' \
      '  protocol: http' \
      '  websocket_port: 6080' \
      '  ssl:' \
      '    require_ssl: false' \
      '    pem_certificate: ""' \
      '    pem_key: ""' \
      > /etc/kasmvnc/kasmvnc.yaml \
 && id ubuntu
