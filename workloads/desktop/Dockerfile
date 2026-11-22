# The SOMA desktop stage: an Openbox desktop on Xorg's dummy screen and the pinned cua-driver, for a SOMA guest.
#
# A SOMA guest has no init system (PID 1 is the guest agent) and its Generation snapshot holds no
# running process, so this stage adds no service unit and starts nothing. It adds what a desktop
# session started after Ready needs, and nothing else:
#
#   at-spi2-core          the accessibility bus cua-driver reads the AT-SPI tree from
#   x11-utils, procps     xwininfo and pgrep, which the session waits on
#   libglib2.0-bin        gdbus, which `cua-driver doctor` probes the accessibility bus with
#   cua-driver            the pinned release binary, checked against its published SHA-256, with
#                         telemetry and the update check off for every caller
#   the session launcher  soma-desktop-session, run once through exec after the machine is Ready
#   the warm list         /etc/soma/warm, so the session starts from cached pages
#
# The base is a desktop image that already carries Xorg with the dummy video driver, Openbox,
# xfce4-terminal, dbus-x11 and the screen configuration at /etc/soma-desktop/xorg.conf.
# Two bases are supported and the stage is the same for both:
#
#   an existing desktop root filesystem, imported with import-rootfs.sh:
#     workloads/desktop/import-rootfs.sh <rootfs.ext4> desktop-rootfs:v17
#     docker build --build-arg BASE_IMAGE=desktop-rootfs:v17 -t soma-desktop:2 workloads/desktop
#
#   the public base built from base.Dockerfile, which needs nothing outside this directory:
#     docker build -f workloads/desktop/base.Dockerfile -t soma-desktop-base:2 workloads/desktop
#     docker build --build-arg BASE_IMAGE=soma-desktop-base:2 -t soma-desktop:2 workloads/desktop
#
# Only instructions the classic builder understands are used, so no BuildKit is needed.
# voxl, the default viewer: H.264 over one WebSocket, decoded with WebCodecs, damage-aware so a
# still screen costs nothing. Built from a pinned commit of tembo/voxl (MIT; the binary links x264,
# so it is distributed under the GPL with its source at that commit). Measured on miosa-host-12 on
# two cores: 60 fps at 1080p even at 5 Mbps / 60 ms, where KasmVNC fell to 27 fps and noVNC to 7.
ARG BASE_IMAGE
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517 AS voxl
ARG VOXL_COMMIT=21f194f9e446e7073abd7bbd52e0b397c3800b42
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git build-essential pkg-config clang \
      libx11-dev libxext-dev libxfixes-dev libxdamage-dev libxtst-dev libxrandr-dev \
      libx264-dev libyuv-dev libopus-dev libpulse-dev xz-utils \
 && curl -fsSL -o /tmp/node.tar.xz https://nodejs.org/dist/v22.21.1/node-v22.21.1-linux-x64.tar.xz \
 && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 && npm install -g esbuild \
 && curl -fsSL https://sh.rustup.rs -o /tmp/rustup.sh && sh /tmp/rustup.sh -y --profile minimal
RUN git clone -q https://github.com/tembo/voxl.git /src/voxl && cd /src/voxl && git checkout -q "$VOXL_COMMIT" \
 && PATH=/root/.cargo/bin:$PATH cargo build --release --locked \
 && install -m 0755 target/release/voxl /usr/local/bin/voxl

FROM ${BASE_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
 && apt-get install -y --no-install-recommends at-spi2-core ca-certificates curl dbus libxkbcommon0 libxi6 \
      libglib2.0-bin procps x11-utils x11-xserver-utils \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && test -x /usr/libexec/at-spi-bus-launcher \
 && test -x /usr/lib/xorg/Xorg && test -f /etc/soma-desktop/xorg.conf \
 && command -v openbox xfce4-terminal dbus-daemon setpriv setsid pgrep xprop xsetroot gdbus

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

# The viewer: Sunshine (captures and encodes the screen, software H.264) and moonlight-web (the
# browser client, relaying over WebSockets), both pinned by the SHA-256 of their release assets.
# udev and the libinput driver let Xorg take the pointer and keyboard Sunshine creates through
# /dev/uinput (the desktop kernel variant has it); the desktop user joins the input group to read
# them. python3 runs the build-time pairing only.
ARG SUNSHINE_VERSION=2026.914.233613
ARG SUNSHINE_SHA256=c38e9c705f650f8705f61702717e99bec34044c5028fcb8f23a08fe041292c21
ARG MOONLIGHT_WEB_VERSION=3.0.2
ARG MOONLIGHT_WEB_SHA256=4b6de5c1c35553e9d35bd67a3343ebd01f67d0a4216116c09b78b12adccc03df
RUN set -e; \
    deb="sunshine_${SUNSHINE_VERSION}-1+ubuntu24.04_amd64.deb"; \
    curl -fsSL --retry 3 -o "/tmp/${deb}" \
      "https://github.com/LizardByte/Sunshine/releases/download/v${SUNSHINE_VERSION}/${deb}"; \
    echo "${SUNSHINE_SHA256}  /tmp/${deb}" | sha256sum -c -; \
    curl -fsSL --retry 3 -o /tmp/moonlight-web.tar.gz \
      "https://github.com/MrCreativ3001/moonlight-web-stream/releases/download/v${MOONLIGHT_WEB_VERSION}/moonlight-web-x86_64-unknown-linux-gnu.tar.gz"; \
    echo "${MOONLIGHT_WEB_SHA256}  /tmp/moonlight-web.tar.gz" | sha256sum -c -; \
    apt-get update; \
    apt-get install -y --no-install-recommends "/tmp/${deb}" udev xserver-xorg-input-libinput python3; \
    mkdir -p /opt/moonlight-web; \
    tar -C /opt/moonlight-web --strip-components=1 -xzf /tmp/moonlight-web.tar.gz; \
    chmod 0755 /opt/moonlight-web/web-server; \
    rm -f "/tmp/${deb}" /tmp/moonlight-web.tar.gz; \
    apt-get clean; \
    find /var/lib/apt/lists -type f -delete; \
    usermod -aG input ubuntu; \
    command -v sunshine; test -x /lib/systemd/systemd-udevd

COPY session/cua-driver-wrapper.sh /usr/local/bin/cua-driver
COPY session/soma-desktop-session.sh /usr/local/sbin/soma-desktop-session
COPY session/soma-desktop-call.sh /usr/local/bin/soma-desktop-call
COPY session/session.sh /usr/local/libexec/soma-desktop/session.sh
COPY session/cua-capabilities.yaml /etc/soma-desktop/cua-capabilities.yaml
COPY viewer/viewer.sh /usr/local/libexec/soma-desktop/viewer.sh
COPY viewer/pair.py /usr/local/libexec/soma-desktop/pair.py
COPY viewer/sunshine.conf /home/ubuntu/.config/sunshine/sunshine.conf

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

# The page-cache warm list: the X server, Openbox, the terminal, the driver and every library they
# link, so a session started after a restore reads them from memory. The agent bounds a list at
# 256 entries.
RUN mkdir -p /etc/soma \
 && { \
      for binary in /usr/lib/xorg/Xorg /usr/lib/xorg/modules/drivers/dummy_drv.so \
                    /usr/bin/openbox /usr/bin/xfce4-terminal \
                    /usr/libexec/at-spi-bus-launcher /usr/libexec/at-spi2-registryd \
                    /usr/bin/dbus-daemon /opt/cua-driver/cua-driver; do \
        [ -e "$binary" ] || continue; \
        echo "$binary"; \
        ldd "$binary" | awk '/=> \//{print $3}'; \
      done; \
    } | awk '!seen[$0]++' | head -256 > /etc/soma/warm \
 && test "$(wc -l < /etc/soma/warm)" -gt 30

# The MIOSA look and the apps a person uses every day: Openbox's MIOSA theme, a top bar and a dock
# (tint2), a static wallpaper (hsetroot), the Yaru GTK and icon themes, Inter and JetBrains Mono,
# Thunar for files, and Google Chrome pinned by the SHA-256 of its package. Chrome is set up for a
# machine that runs for weeks with accounts signed in: it restores the last session, never asks to
# be the default or shows a first run, and keeps logins without a keyring prompt.
ARG CHROME_VERSION=158.0.8127.36-1
ARG CHROME_SHA256=4696bce00b128f385a870ba2eb6eede61698f22374ea0d8e09af8a1c6543d397
RUN set -e; \
    deb="google-chrome-stable_${CHROME_VERSION}_amd64.deb"; \
    curl -fsSL --retry 3 -o "/tmp/${deb}" \
      "https://dl.google.com/linux/chrome/deb/pool/main/g/google-chrome-stable/${deb}"; \
    echo "${CHROME_SHA256}  /tmp/${deb}" | sha256sum -c -; \
    apt-get update; \
    apt-get install -y --no-install-recommends "/tmp/${deb}" tint2 hsetroot thunar picom \
      fonts-inter fonts-jetbrains-mono yaru-theme-gtk yaru-theme-icon; \
    rm -f "/tmp/${deb}" /etc/apt/sources.list.d/google-chrome.list; \
    apt-get clean; \
    find /var/lib/apt/lists -type f -delete; \
    command -v google-chrome-stable tint2 hsetroot thunar picom
COPY --from=voxl /usr/local/bin/voxl /usr/local/bin/voxl
RUN apt-get update \
 && apt-get install -y --no-install-recommends libx264-164 libyuv0 libopus0 libpulse0 \
 && apt-get clean && find /var/lib/apt/lists -type f -delete \
 && voxl --help | head -1

# KasmVNC, the screen and the fallback viewer (see session.sh): pinned by the SHA-256 of its release
# package, with its web client served on the viewer port.
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
    command -v Xkasmvnc; \
    mkdir -p /etc/kasmvnc; \
    printf '%s\n' 'network:' '  protocol: http' '  websocket_port: 6082' '  ssl:' \
      '    require_ssl: false' '    pem_certificate: ""' '    pem_key: ""' > /etc/kasmvnc/kasmvnc.yaml
COPY look/openbox/themerc /usr/share/themes/MIOSA/openbox-3/themerc
COPY look/openbox/rc.xml /etc/soma-desktop/openbox.xml
COPY look/openbox/menu.xml /etc/soma-desktop/menu.xml
COPY look/tint2/ /etc/soma-desktop/tint2/
COPY look/terminal/terminalrc /home/ubuntu/.config/xfce4/terminal/terminalrc
COPY look/gtk/settings.ini /home/ubuntu/.config/gtk-3.0/settings.ini
COPY look/chrome/miosa.json /etc/opt/chrome/policies/managed/miosa.json
COPY look/brand/miosa-icon.png /usr/share/soma-desktop/miosa-icon.png
COPY look/compositor/picom.conf /etc/soma-desktop/picom.conf
COPY look/wallpaper/blue-hour.jpg /usr/share/soma-desktop/blue-hour.jpg
COPY look/viewer/apps.json /home/ubuntu/.config/sunshine/apps.json

# A real desktop and an Apple-style dock: PCManFM draws the desktop (icons, folders, drag-to-select,
# the right-click menu, wallpaper picker), Plank is the dock, WhiteSur is the icon set, and the
# MIOSA wallpapers come in dark and light. WhiteSur is pinned by the SHA-256 of its release.
ARG WHITESUR_TAG=2026-09-10
ARG WHITESUR_SHA256=406c9cd59705583f1754b0eaca96cc48bafda042b88ef143f16d8ae1820ecd95
RUN set -e; \
    apt-get update; \
    apt-get install -y --no-install-recommends pcmanfm plank gsimplecal dconf-cli dconf-service \
      dconf-gsettings-backend gvfs librsvg2-common; \
    apt-get clean; \
    find /var/lib/apt/lists -type f -delete; \
    curl -fsSL --retry 3 -o /tmp/whitesur.tar.gz \
      "https://github.com/vinceliuice/WhiteSur-icon-theme/archive/refs/tags/${WHITESUR_TAG}.tar.gz"; \
    echo "${WHITESUR_SHA256}  /tmp/whitesur.tar.gz" | sha256sum -c -; \
    mkdir -p /tmp/whitesur; tar -xzf /tmp/whitesur.tar.gz -C /tmp/whitesur --strip-components=1; \
    (cd /tmp/whitesur && ./install.sh -d /usr/share/icons -t default >/dev/null); \
    rm -r -f /tmp/whitesur /tmp/whitesur.tar.gz; \
    test -d /usr/share/icons/WhiteSur-dark; \
    command -v pcmanfm plank gsimplecal
COPY look/backgrounds/miosa-dark.jpg look/backgrounds/miosa-light.jpg /usr/share/backgrounds/miosa/
COPY look/desktop/pcmanfm.conf look/desktop/desktop-items-0.conf /etc/xdg/pcmanfm/miosa/
COPY look/desktop/libfm.conf /etc/xdg/libfm/libfm.conf
COPY look/desktop/miosa-desktop-settings.desktop /usr/share/applications/miosa-desktop-settings.desktop
COPY look/dock/MIOSA/dock.theme /usr/share/plank/themes/MIOSA/dock.theme
COPY look/dock/dconf-plank /etc/dconf/db/local.d/10-plank
COPY look/dock/*.dockitem /home/ubuntu/.config/plank/dock1/launchers/
RUN mkdir -p /etc/dconf/profile \
 && printf 'user-db:user\nsystem-db:local\n' > /etc/dconf/profile/user \
 && dconf update \
 && mkdir -p /home/ubuntu/Desktop /home/ubuntu/Documents /home/ubuntu/Downloads \
 && for app in google-chrome xfce4-terminal pcmanfm miosa-desktop-settings; do \
      install -m 0755 "/usr/share/applications/${app}.desktop" "/home/ubuntu/Desktop/${app}.desktop"; \
    done \
 && ln -s /home/ubuntu/Documents /home/ubuntu/Desktop/Documents \
 && ln -s /home/ubuntu/Downloads /home/ubuntu/Desktop/Downloads \
 && chown -R ubuntu:ubuntu /home/ubuntu/Desktop /home/ubuntu/Documents /home/ubuntu/Downloads /home/ubuntu/.config
RUN test -s /usr/share/soma-desktop/blue-hour.jpg \
 && printf '#!/bin/sh\nexec /usr/bin/google-chrome-stable --password-store=basic --no-first-run --no-default-browser-check "$@"\n' \
      > /usr/local/bin/google-chrome \
 && chmod 0755 /usr/local/bin/google-chrome \
 && sed -i 's|^Exec=/usr/bin/google-chrome-stable|Exec=/usr/local/bin/google-chrome|' \
      /usr/share/applications/google-chrome.desktop \
 && grep -q '^Exec=/usr/local/bin/google-chrome' /usr/share/applications/google-chrome.desktop \
 && test -f /usr/share/applications/xfce4-terminal.desktop \
 && test -f /usr/share/applications/thunar.desktop \
 && cp /home/ubuntu/.config/gtk-3.0/settings.ini /etc/gtk-3.0/settings.ini \
 && chown -R ubuntu:ubuntu /home/ubuntu/.config

# The viewer opens straight into this desktop: its front door finds the host and the Desktop app and
# goes to the stream, every page defaults to WebSockets (WebRTC cannot cross the SOMA tunnel), and
# the page carries the MIOSA name. The default is patched in each bundle that sets it, and the build
# fails if the bundles change shape under a new moonlight-web.
COPY look/viewer/index.html /opt/moonlight-web/static/index.html
COPY look/brand/miosa-icon.png /opt/moonlight-web/static/miosa-icon.png
COPY look/brand/miosa-logo.png /opt/moonlight-web/static/miosa-logo.png
RUN cd /opt/moonlight-web/static \
 && test "$(grep -l 'dataTransport:"auto"' ./*.js | wc -l)" -eq 4 \
 && sed -i 's/dataTransport:"auto"/dataTransport:"websocket"/' ./*.js \
 && test "$(grep -l 'dataTransport:"auto"' ./*.js | wc -l)" -eq 0 \
 && sed -i 's|<title>Stream</title>|<title>MIOSA Desktop</title>|; s|href=resources/moonlight.svg|href=miosa-icon.png|; s|href=moonlight.svg|href=miosa-icon.png|' stream.html \
 && grep -q '<title>MIOSA Desktop</title>' stream.html

# The viewer's own state, made once: moonlight-web's configuration (WebSockets only, so no public
# STUN server is ever asked; its data in the desktop user's home), then the pairing itself.
RUN chmod 0755 /usr/local/libexec/soma-desktop/viewer.sh /usr/local/libexec/soma-desktop/pair.py \
 && mkdir -p /home/ubuntu/.config/moonlight-web \
 && mkdir -p /tmp/mlw-gen && cd /tmp/mlw-gen && /opt/moonlight-web/web-server config generate >/dev/null \
 && python3 -c 'import json; c = json.load(open("/tmp/mlw-gen/server/config.json")); c["data_storage"]["path"] = "/home/ubuntu/.config/moonlight-web/data.json"; c["webrtc"]["ice_servers"] = []; json.dump(c, open("/home/ubuntu/.config/moonlight-web/config.json", "w"), indent=2)' \
 && cd / && rm -r -f /tmp/mlw-gen \
 && chown -R ubuntu:ubuntu /home/ubuntu/.config \
 && setpriv --reuid="$(id -u ubuntu)" --regid="$(id -g ubuntu)" --init-groups \
      env HOME=/home/ubuntu LANG=C.UTF-8 python3 /usr/local/libexec/soma-desktop/pair.py \
 && test -s /home/ubuntu/.config/moonlight-web/data.json \
 && grep -q '"named_devices"' /home/ubuntu/.config/sunshine/sunshine_state.json \
 && rm -rf /tmp/.X11-unix /tmp/.X0-lock /tmp/soma-viewer-pair
