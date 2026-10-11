#!/bin/bash
# Installs Docker CE from Docker's official apt repository at exact pinned
# versions and wires it for socket activation. Shared by every MIOSA guest
# image that offers Docker; the identical file lives in miosa-compute under
# templates/_shared/install-docker-ce.sh (a test there keeps the copies equal).
#
# Versions are exact upstream pins, never ranges. The script appends the distro
# suffix itself, e.g. "5:29.9.0-1~debian.12~bookworm", so one pin set serves
# Debian 12/13 and Ubuntu 24.04 images. The defaults below are the pin set; a
# caller may override them through the environment (miosa-sandbox passes the
# values recorded in image-source.json, and a test keeps both equal).
#   DOCKER_CE_VERSION DOCKER_CLI_VERSION CONTAINERD_VERSION
#   DOCKER_BUILDX_VERSION DOCKER_COMPOSE_VERSION
# Optional:
#   DOCKER_SANDBOX_USER      user added to the docker group (default: none)
#   DOCKER_SOCKET_ACTIVATED  1 (default) leaves docker.service and
#                            containerd.service disabled so the first client
#                            connection to /run/docker.sock starts them;
#                            0 enables both at boot.
set -euo pipefail

: "${DOCKER_CE_VERSION:=29.9.0}" "${DOCKER_CLI_VERSION:=29.9.0}" "${CONTAINERD_VERSION:=2.4.1-2}"
: "${DOCKER_BUILDX_VERSION:=0.38.0-1}" "${DOCKER_COMPOSE_VERSION:=5.6.0-1}"

# Published at https://docs.docker.com/engine/install/debian/ ; checked against
# the downloaded key so a swapped key fails the build instead of being trusted.
DOCKER_APT_KEY_FPR="9DC858229FC7DD38854AE2D88D81803C0EBFCD88"

export DEBIAN_FRONTEND=noninteractive
# shellcheck disable=SC1091
. /etc/os-release
case "$ID" in
  debian | ubuntu) ;;
  *)
    echo "unsupported distro for Docker CE: $ID" >&2
    exit 1
    ;;
esac
suffix="~${ID}.${VERSION_ID}~${VERSION_CODENAME}"
arch="$(dpkg --print-architecture)"

# Slim bases (the docker-deploy runtime) ship neither gpg nor curl.
if ! command -v gpg >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1; then
  apt-get update
  apt-get install -y --no-install-recommends ca-certificates curl gnupg
fi

install -d -m 0755 /etc/apt/keyrings
curl -fsSL --retry 3 "https://download.docker.com/linux/${ID}/gpg" -o /tmp/docker.gpg
actual="$(gpg --show-keys --with-colons /tmp/docker.gpg | awk -F: '/^fpr:/ {print $10; exit}')"
if [ "$actual" != "$DOCKER_APT_KEY_FPR" ]; then
  echo "Docker apt key fingerprint mismatch: $actual" >&2
  exit 1
fi
gpg --dearmor < /tmp/docker.gpg > /etc/apt/keyrings/docker.gpg
chmod 0644 /etc/apt/keyrings/docker.gpg
rm -f /tmp/docker.gpg

printf 'deb [arch=%s signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/%s %s stable\n' \
  "$arch" "$ID" "$VERSION_CODENAME" > /etc/apt/sources.list.d/docker.list

# A distro engine must never coexist with Docker CE: two dockerd binaries
# means two sockets and two data roots.
apt-get remove -y docker.io docker-compose docker-doc podman-docker containerd runc 2>/dev/null || true

# iptables is the one hard runtime dependency that --no-install-recommends
# installs skip; pigz speeds layer extraction; xz-utils unpacks xz layers.
apt-get update
apt-get install -y --no-install-recommends \
  iptables pigz xz-utils \
  "docker-ce=5:${DOCKER_CE_VERSION}-1${suffix}" \
  "docker-ce-cli=5:${DOCKER_CLI_VERSION}-1${suffix}" \
  "containerd.io=${CONTAINERD_VERSION}${suffix}" \
  "docker-buildx-plugin=${DOCKER_BUILDX_VERSION}${suffix}" \
  "docker-compose-plugin=${DOCKER_COMPOSE_VERSION}${suffix}"
apt-mark hold docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin > /dev/null

getent group docker > /dev/null || groupadd --system docker
if [ -n "${DOCKER_SANDBOX_USER:-}" ]; then
  usermod -aG docker "$DOCKER_SANDBOX_USER"
fi

if [ "${DOCKER_SOCKET_ACTIVATED:-1}" = "1" ]; then
  # docker.socket stays enabled (sockets.target). docker.service Requires the
  # socket and Wants containerd.service, so the first connection starts both.
  systemctl disable docker.service containerd.service > /dev/null 2>&1 || true
  systemctl enable docker.socket > /dev/null 2>&1
else
  systemctl enable docker.service containerd.service docker.socket > /dev/null 2>&1
fi
