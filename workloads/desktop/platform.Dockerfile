# Add the MIOSA desktop API service to the SOMA desktop.
# Build desktopd from MIOSA Compute's envd module for linux/amd64 with CGO disabled.
# The build context is a directory containing that executable as desktopd.
ARG DESKTOP_IMAGE
FROM ${DESKTOP_IMAGE}
COPY desktopd /usr/local/bin/desktopd
RUN chmod 0755 /usr/local/bin/desktopd
