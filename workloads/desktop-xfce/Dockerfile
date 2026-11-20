# The SOMA desktop stage: an Xfce desktop with KasmVNC and the pinned cua-driver, for a SOMA guest.
#
# A SOMA guest has no init system (PID 1 is the guest agent) and its Generation snapshot holds no
# running process, so this stage adds no service unit and starts nothing. It adds what a desktop
# session started after Ready needs, and nothing else:
#
#   at-spi2-core          the accessibility bus cua-driver reads the AT-SPI tree from
#   x11-utils, procps     xwininfo and pgrep, which the session waits on
#   cua-driver            the pinned release binary, checked against its published SHA-256, with
#                         telemetry and the update check off for every caller
#   the session launcher  soma-desktop-session, run once through exec after the machine is Ready
#   the warm list         /etc/soma/warm, so the session starts from cached pages
#
# The base is a desktop image that already carries Xfce 4, xfce4-terminal, dbus-x11 and KasmVNC
# 1.4.0 (Xkasmvnc). Two bases are supported and the stage is the same for both:
#
#   an existing desktop root filesystem, imported with import-rootfs.sh:
#     workloads/desktop-xfce/import-rootfs.sh <rootfs.ext4> desktop-rootfs:v17
#     docker build --build-arg BASE_IMAGE=desktop-rootfs:v17 -t soma-desktop-xfce:1 workloads/desktop-xfce
#
#   the public base built from base.Dockerfile, which needs nothing outside this directory:
#     docker build -f workloads/desktop-xfce/base.Dockerfile -t soma-desktop-base:1 workloads/desktop-xfce
#     docker build --build-arg BASE_IMAGE=soma-desktop-base:1 -t soma-desktop-xfce:1 workloads/desktop-xfce
#
# Only instructions the classic builder understands are used, so no BuildKit is needed.
ARG BASE_IMAGE
FROM ${BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
 && apt-get install -y --no-install-recommends at-spi2-core ca-certificates curl dbus libxkbcommon0 libxi6 \
      procps x11-utils \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && test -x /usr/libexec/at-spi-bus-launcher \
 && command -v Xkasmvnc startxfce4 xfce4-terminal dbus-daemon setpriv setsid pgrep xwininfo

# cua-driver, pinned. 0.55.0 is the newest release with published binaries, SHA256SUMS and
# Sigstore bundles: the cua-driver-rs-v0.56.0 tag (trycua/cua 9c75001355, 2026-11-20) has no
# release assets. Only the MIT driver is installed: no cua-spacesd, no streaming component and no
# perception extension, and the build fails if one appears.
ARG CUA_DRIVER_VERSION=0.55.0
ARG CUA_DRIVER_SHA256=fd863defadb13573d53fd327e7d09268460a658224651329cb0b857b0c8752ac
RUN set -e; \
    archive="cua-driver-rs-${CUA_DRIVER_VERSION}-linux-x86_64"; \
    curl -fsSL --retry 3 -o /tmp/cua-driver.tar.gz \
      "https://github.com/trycua/cua/releases/download/cua-driver-rs-v${CUA_DRIVER_VERSION}/${archive}.tar.gz"; \
    echo "${CUA_DRIVER_SHA256}  /tmp/cua-driver.tar.gz" | sha256sum -c -; \
    mkdir -p /tmp/cua-driver /opt/cua-driver; \
    tar -C /tmp/cua-driver -xzf /tmp/cua-driver.tar.gz; \
    for file in cua-driver LICENSE THIRD_PARTY_NOTICES.md; do \
      install -m 0644 "/tmp/cua-driver/${archive}/${file}" "/opt/cua-driver/${file}"; \
    done; \
    chmod 0755 /opt/cua-driver/cua-driver; \
    printf '%s %s\n' "${CUA_DRIVER_VERSION}" "${CUA_DRIVER_SHA256}" > /opt/cua-driver/PINNED; \
    rm -rf /tmp/cua-driver /tmp/cua-driver.tar.gz; \
    if find / -xdev \( -name 'cua-spacesd*' -o -name '*cua-perception*' -o -name 'cua-relay*' \) -print -quit | grep -q .; then \
      echo 'a non-MIT cua component is in the image' >&2; exit 1; \
    fi

COPY session/cua-driver-wrapper.sh /usr/local/bin/cua-driver
COPY session/soma-desktop-session.sh /usr/local/sbin/soma-desktop-session
COPY session/soma-desktop-call.sh /usr/local/bin/soma-desktop-call
COPY session/session.sh /usr/local/libexec/soma-desktop/session.sh
COPY session/cua-capabilities.yaml /etc/soma-desktop/cua-capabilities.yaml

# Telemetry and update checks off in the persisted preference too, for the desktop user and root,
# and asserted: the wrapper must export both switches and both files must say false.
RUN chmod 0755 /usr/local/bin/cua-driver /usr/local/sbin/soma-desktop-session \
      /usr/local/bin/soma-desktop-call /usr/local/libexec/soma-desktop/session.sh \
 && chmod 0644 /etc/soma-desktop/cua-capabilities.yaml \
 && for home in /root /home/ubuntu; do \
      mkdir -p "$home/.cua-driver"; \
      printf '{"telemetry_enabled": false, "update_check_enabled": false}\n' > "$home/.cua-driver/config.json"; \
    done \
 && chown -R ubuntu:ubuntu /home/ubuntu/.cua-driver \
 && grep -qx 'export CUA_DRIVER_RS_TELEMETRY_ENABLED=0' /usr/local/bin/cua-driver \
 && grep -qx 'export CUA_DRIVER_RS_UPDATE_CHECK=0' /usr/local/bin/cua-driver \
 && grep -q '"telemetry_enabled": false' /home/ubuntu/.cua-driver/config.json \
 && cua-driver --version

# One machine id per Instance. The guest agent writes /etc/machine-id for every Instance; D-Bus
# reads /var/lib/dbus/machine-id, which in a captured image would otherwise be one id shared by
# every machine restored from it.
#
# A swap file baked into a base for another runtime is removed: a SOMA guest never enables swap,
# and the file would only make the shared lower larger.
RUN rm -f /var/lib/dbus/machine-id /swapfile \
 && mkdir -p /var/lib/dbus \
 && ln -s /etc/machine-id /var/lib/dbus/machine-id

# The page-cache warm list: the X server, Xfce, the terminal, the driver and every library they
# link, so a session started after a restore reads them from memory. The agent bounds a list at
# 256 entries.
RUN mkdir -p /etc/soma \
 && { \
      for binary in /usr/bin/Xkasmvnc /usr/bin/xfwm4 /usr/bin/xfce4-panel /usr/bin/xfdesktop \
                    /usr/bin/xfce4-session /usr/bin/xfsettingsd /usr/bin/xfce4-terminal \
                    /usr/libexec/at-spi-bus-launcher /usr/libexec/at-spi2-registryd \
                    /usr/bin/dbus-daemon /opt/cua-driver/cua-driver; do \
        [ -e "$binary" ] || continue; \
        echo "$binary"; \
        ldd "$binary" | awk '/=> \//{print $3}'; \
      done; \
    } | awk '!seen[$0]++' | head -256 > /etc/soma/warm \
 && test "$(wc -l < /etc/soma/warm)" -gt 50
