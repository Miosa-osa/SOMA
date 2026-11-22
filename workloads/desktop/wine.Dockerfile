# The SOMA desktop with Wine: Windows programs as ordinary windows on the Linux desktop, driven by
# the same cua-driver. A layer on top of the desktop stage, so a plain desktop does not carry it.
#
#   docker build -f workloads/desktop/wine.Dockerfile --build-arg DESKTOP_IMAGE=soma-desktop:2 \
#     -t soma-desktop-wine:2 workloads/desktop
#
# Wine is Ubuntu 24.04's own package (both the 64-bit and the 32-bit loader, since most Windows
# installers are 32-bit), from the archive of the pinned base. The guest kernel has no
# binfmt_misc, so a program is started as `wine program.exe`, never by its name alone. The prefix
# is created at build time for the desktop user, so the first program does not wait for it;
# wineserver writes the registry only when it exits, hence the wait.
ARG DESKTOP_IMAGE
FROM ${DESKTOP_IMAGE}

ENV DEBIAN_FRONTEND=noninteractive

RUN dpkg --add-architecture i386 \
 && apt-get update \
 && apt-get install -y --no-install-recommends wine wine64 wine32:i386 \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && wine --version

# The prefix, made once with no display (wineboot needs none), owned by the desktop user.
RUN setpriv --reuid="$(id -u ubuntu)" --regid="$(id -g ubuntu)" --init-groups \
      env HOME=/home/ubuntu WINEPREFIX=/home/ubuntu/.wine WINEDEBUG=-all DISPLAY= \
      sh -c 'wineboot --init && wineserver -w' \
 && test -f /home/ubuntu/.wine/system.reg
