# A public desktop base for the SOMA desktop stage (Dockerfile in this directory).
#
# It carries what the stage expects of any desktop base, built from public sources only:
# Ubuntu 24.04, Xorg with the dummy video driver as the screen (configured in xorg/), Openbox as
# the window manager, xfce4-terminal as the one application, D-Bus with the X11 helpers, and the X
# utilities the session waits with. There is no VNC server: the screen is for the agent, through
# cua-driver, and a human viewer is a separate stream added on top of the same display.
#
#   docker build -f workloads/desktop/base.Dockerfile -t soma-desktop-base:2 workloads/desktop
#
# Pin: the base by index digest (the same reviewed ubuntu:24.04 the kernel builder and the netd
# live tests use). Packages come from that release's archive.
FROM ubuntu@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517

ENV DEBIAN_FRONTEND=noninteractive
ENV LANG=C.UTF-8

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl dbus dbus-x11 fonts-dejavu-core procps sudo xauth \
      xserver-xorg-core xserver-xorg-video-dummy openbox xfce4-terminal \
      x11-utils x11-xserver-utils \
 && apt-get clean \
 && find /var/lib/apt/lists -type f -delete \
 && test -x /usr/lib/xorg/Xorg \
 && test -f /usr/lib/xorg/modules/drivers/dummy_drv.so \
 && command -v openbox xfce4-terminal \
 && id ubuntu

COPY xorg/10-dummy-screen.conf /etc/soma-desktop/xorg.conf
