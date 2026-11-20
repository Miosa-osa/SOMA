# SOMA desktop workload: Xfce on KasmVNC's X server, driven by cua-driver.
#
#   docker build --platform=linux/amd64 -t soma-desktop:m1 workloads/desktop
#
# A SOMA guest has no init system (PID 1 is the guest agent) and its Generation snapshot holds no
# running process, so nothing here is a service: the session is started after Ready by one exec of
# `desktop-session` (files/desktop-session). Only classic-builder instructions are used, so no
# BuildKit is needed. Every download is pinned by SHA-256; see README.md for where each pin is from.
FROM ubuntu@sha256:534baea6a22c03a63003dbc8dbe78fe34bc0d7e595d9a9dc9834884ff530eb55

ARG KASMVNC_VERSION=1.4.0
ARG KASMVNC_SHA256=12bac6014149c5fdee75f0d403785aaa3e5dd4ea222de73253a5d4181bc9567e
ARG CUA_DRIVER_VERSION=0.55.0
ARG CUA_DRIVER_SHA256=fd863defadb13573d53fd327e7d09268460a658224651329cb0b857b0c8752ac
ENV DEBIAN_FRONTEND=noninteractive LANG=C.UTF-8 LC_ALL=C.UTF-8

# libglib2.0-bin carries gdbus and gsettings: `cua-driver doctor` probes the accessibility bus with
# gdbus, and the session turns toolkit accessibility on with gsettings.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl python3 procps iproute2 util-linux \
      xfce4-session xfwm4 xfce4-panel xfdesktop4 xfce4-settings xfce4-terminal thunar \
      dbus dbus-x11 at-spi2-core libatk-adaptor dconf-cli gsettings-desktop-schemas libglib2.0-bin \
      x11-utils x11-xserver-utils xdotool xclip wmctrl \
      fonts-dejavu-core fonts-liberation \
      libx11-6 libxi6 libxtst6 libxext6 libxres1 libwayland-client0 libxkbcommon0 \
 && curl -fsSL --retry 3 -o /tmp/kasmvnc.deb \
      "https://github.com/kasmtech/KasmVNC/releases/download/v${KASMVNC_VERSION}/kasmvncserver_noble_${KASMVNC_VERSION}_amd64.deb" \
 && echo "${KASMVNC_SHA256}  /tmp/kasmvnc.deb" | sha256sum -c - \
 && apt-get install -y --no-install-recommends /tmp/kasmvnc.deb \
 && rm -f /tmp/kasmvnc.deb \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && command -v Xkasmvnc gdbus gsettings runuser setsid xwininfo \
 && id ubuntu

# The driver comes from its release archive, never from an installer script. The full archive is
# used, not the bare `-binary` one, because only the full one carries the MIT licence and the
# third-party notices that must ship with the binary; both are lines of the release's SHA256SUMS.
# Only the driver itself is installed: no cua-spacesd, streaming component or perception extension
# (licensed FSL or AGPL), and the build fails if any appears.
RUN set -e; \
    archive="cua-driver-rs-${CUA_DRIVER_VERSION}-linux-x86_64"; \
    curl -fsSL --retry 3 -o /tmp/cua.tgz \
      "https://github.com/trycua/cua/releases/download/cua-driver-rs-v${CUA_DRIVER_VERSION}/${archive}.tar.gz"; \
    echo "${CUA_DRIVER_SHA256}  /tmp/cua.tgz" | sha256sum -c -; \
    mkdir -p /tmp/cua /opt/cua-driver; \
    tar -xzf /tmp/cua.tgz -C /tmp/cua; \
    install -m 0755 "/tmp/cua/${archive}/cua-driver" /opt/cua-driver/cua-driver; \
    install -m 0644 "/tmp/cua/${archive}/LICENSE" "/tmp/cua/${archive}/THIRD_PARTY_NOTICES.md" /opt/cua-driver/; \
    rm -rf /tmp/cua /tmp/cua.tgz; \
    if ldd /opt/cua-driver/cua-driver | grep 'not found'; then exit 1; fi; \
    CUA_DRIVER_RS_TELEMETRY_ENABLED=0 CUA_DRIVER_RS_UPDATE_CHECK=0 /opt/cua-driver/cua-driver --version \
      | grep -F "${CUA_DRIVER_VERSION}"; \
    if find / -xdev \( -name 'cua-spacesd*' -o -name 'cua-relay*' -o -name '*cua-perception*' \) -print | grep .; then \
      exit 1; \
    fi

COPY files/kasmvnc.yaml /etc/kasmvnc/kasmvnc.yaml
COPY files/desktop-session /usr/local/sbin/desktop-session
COPY files/desktop-session-user /usr/local/lib/desktop/desktop-session-user
COPY files/cua files/cua-call /usr/local/bin/

# Telemetry and the update check off in the driver's persisted preference too, for `ubuntu` and
# root, so a caller that bypasses `cua` and the session still gets both off. D-Bus reads its
# machine id from /var/lib/dbus; the guest agent writes /etc/machine-id fresh for every Instance,
# so the one is a link to the other rather than a copy shared by every machine restored from it.
RUN chmod 0755 /usr/local/sbin/desktop-session /usr/local/lib/desktop/desktop-session-user \
      /usr/local/bin/cua /usr/local/bin/cua-call \
 && chmod 0644 /etc/kasmvnc/kasmvnc.yaml \
 && usermod -aG ssl-cert ubuntu \
 && for home in /root /home/ubuntu; do \
      mkdir -p "$home/.cua-driver"; \
      printf '{"telemetry_enabled": false, "update_check_enabled": false}\n' > "$home/.cua-driver/config.json"; \
    done \
 && chown -R ubuntu:ubuntu /home/ubuntu/.cua-driver \
 && mkdir -p /var/lib/dbus \
 && rm -f /var/lib/dbus/machine-id \
 && ln -s /etc/machine-id /var/lib/dbus/machine-id

# Page-cache warm list (/etc/soma/warm): read by the guest agent before capture, at most 256 entries.
RUN mkdir -p /etc/soma \
 && { for binary in "$(command -v Xkasmvnc)" /usr/bin/xfce4-session /usr/bin/xfwm4 /usr/bin/xfce4-panel \
               /usr/bin/xfdesktop /usr/bin/xfce4-terminal /usr/bin/dbus-daemon \
               /usr/libexec/at-spi-bus-launcher /opt/cua-driver/cua-driver; do \
        echo "$binary"; ldd "$binary" | awk '/=> \//{print $3}'; done; echo /usr/bin/python3; } \
    | awk '!seen[$0]++' | head -256 > /etc/soma/warm \
 && test "$(wc -l < /etc/soma/warm)" -gt 50
