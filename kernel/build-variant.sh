#!/usr/bin/env bash
#
# Reproducible build of a guest kernel variant: the machine contract v1 kernel with one fragment's
# additions built in, from the same pinned Linux source, builder image, and toolchain as
# `kernel/build.sh`. The version 1 kernel and `config-x86_64-soma-v1` are untouched; a variant
# only adds facilities, so a guest that never uses them behaves exactly as the version 1 kernel.
#
#   large    soma-large.fragment: compressed swap (D3) and transparent huge pages (D4)
#   desktop  soma-desktop.fragment: the input subsystem and uinput, for a desktop viewer
#
# Inputs:  source.json, soma-v1.fragment, soma-<variant>.fragment, required-config.txt,
#          Dockerfile (config-x86_64-soma-v1 is NOT read). Outputs: out-<variant>/vmlinux-<ver>-
#          soma-<variant>, its .sha256, out-<variant>/final.config, config-x86_64-soma-<variant>,
#          out-<variant>/build.log. Every `CONFIG_*=y` line of the fragment must hold afterwards.
#
# Usage (from the repository root or the kernel directory):
#   kernel/build-variant.sh <variant>                 build the variant kernel
#   kernel/build-variant.sh <variant> regen-config    regenerate its config from the fragments
set -euo pipefail

KDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
VARIANT="${VARIANT:-}"
[ "${1:-}" = "--inner" ] || { VARIANT="${1:-}"; shift || true; }
case "$VARIANT" in
  large|desktop) ;;
  *) echo "usage: $0 large|desktop [build|regen-config]" >&2; exit 64 ;;
esac
OUT="$KDIR/out-$VARIANT"
CONFIG_NAME="config-x86_64-soma-$VARIANT"
SUFFIX="soma-$VARIANT"
FRAGMENT="soma-$VARIANT.fragment"

json_field() {
  python3 - "$1" "$2" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
for part in sys.argv[2].split("."):
    value = value[part]
print(value)
PY
}

inner() {
  local mode="$1"
  local version tarball srcdir cfg jobs
  version="$(json_field /work/source.json kernel_version)"
  tarball="/work/out/src/linux-${version}.tar.xz"
  # The source tree and object files live on the container's own filesystem, not the bind
  # mount: extracting a six-hundred-megabyte tree onto a host-mounted directory races with the
  # mount's own rename handling and fails part way, and the objects are not outputs anyway.
  srcdir="/tmp/build/linux-${version}"
  cfg="/work/${CONFIG_NAME}"
  jobs="${SOMA_KERNEL_JOBS:-$(nproc)}"

  local want_gcc want_ld want_make
  want_gcc="$(json_field /work/source.json toolchain.gcc)"
  want_ld="$(json_field /work/source.json toolchain.ld)"
  want_make="$(json_field /work/source.json toolchain.make)"
  [ "$(gcc -dumpfullversion)" = "$want_gcc" ] || { echo "toolchain drift: gcc != $want_gcc" >&2; exit 2; }
  ld --version | head -1 | grep -Fq " $want_ld" || { echo "toolchain drift: ld != $want_ld" >&2; exit 2; }
  [ "$(make --version | head -1 | awk '{print $3}')" = "$want_make" ] || { echo "toolchain drift: make != $want_make" >&2; exit 2; }

  rm -rf /tmp/build
  mkdir -p /tmp/build
  tar -C /tmp/build -xJf "$tarball"
  cd "$srcdir"

  make -s x86_64_defconfig
  scripts/kconfig/merge_config.sh -m .config /work/soma-v1.fragment "/work/$FRAGMENT" >/dev/null
  make -s olddefconfig
  cp .config "$cfg"
  if [ "$mode" = "regen-config" ]; then
    echo "regenerated ${CONFIG_NAME}"
    return 0
  fi
  python3 /work/verify-config.py --pinned "$cfg" --final .config --required /work/required-config.txt
  for symbol in $(sed -n 's/^\(CONFIG_[A-Z0-9_]*\)=y$/\1/p' "/work/$FRAGMENT"); do
    grep -q "^${symbol}=y" .config || { echo "the fragment did not enable ${symbol}" >&2; exit 2; }
  done
  cp .config "/work/out-$VARIANT/final.config"

  export KBUILD_BUILD_TIMESTAMP KBUILD_BUILD_USER KBUILD_BUILD_HOST KBUILD_BUILD_VERSION SOURCE_DATE_EPOCH
  local start end
  start="$(date +%s.%N)"
  make -j"$jobs" vmlinux
  end="$(date +%s.%N)"
  awk -v s="$start" -v e="$end" -v j="$jobs" 'BEGIN{printf "make vmlinux wall seconds: %.1f (jobs=%d)\n", e-s, j}'
  cp vmlinux "/work/out-$VARIANT/vmlinux-${version}-${SUFFIX}"
  ( cd "/work/out-$VARIANT" && sha256sum "vmlinux-${version}-${SUFFIX}" > "vmlinux-${version}-${SUFFIX}.sha256" )
}

host() {
  local mode="$1"
  local version url want_sha tarball base_digest image_id
  version="$(json_field "$KDIR/source.json" kernel_version)"
  url="$(json_field "$KDIR/source.json" tarball_url)"
  want_sha="$(json_field "$KDIR/source.json" tarball_sha256)"
  base_digest="$(json_field "$KDIR/source.json" builder_base_image)"
  mkdir -p "$KDIR/out/src" "$OUT"
  tarball="$KDIR/out/src/linux-${version}.tar.xz"
  if [ ! -f "$tarball" ] || [ "$(sha256sum "$tarball" | awk '{print $1}')" != "$want_sha" ]; then
    echo "downloading $url"
    curl -fsSL --retry 3 -o "$tarball.part" "$url"
    mv "$tarball.part" "$tarball"
  fi
  [ "$(sha256sum "$tarball" | awk '{print $1}')" = "$want_sha" ] || { echo "source tarball digest mismatch" >&2; exit 2; }

  image_id="$(docker build -q --platform linux/amd64 -f "$KDIR/Dockerfile" "$KDIR")"
  echo "builder image: $image_id"

  local env_ts env_user env_host env_ver env_epoch
  env_ts="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_TIMESTAMP)"
  env_user="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_USER)"
  env_host="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_HOST)"
  env_ver="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_VERSION)"
  env_epoch="$(json_field "$KDIR/source.json" reproducible_env.SOURCE_DATE_EPOCH)"

  docker run --rm --platform linux/amd64 --network none \
    -u "$(id -u):$(id -g)" \
    -v "$KDIR:/work" -w /work \
    -e KBUILD_BUILD_TIMESTAMP="$env_ts" \
    -e KBUILD_BUILD_USER="$env_user" \
    -e KBUILD_BUILD_HOST="$env_host" \
    -e KBUILD_BUILD_VERSION="$env_ver" \
    -e SOURCE_DATE_EPOCH="$env_epoch" \
    -e SOMA_KERNEL_JOBS="${SOMA_KERNEL_JOBS:-}" \
    -e VARIANT="$VARIANT" \
    "$image_id" bash /work/build-variant.sh --inner "$mode" 2>&1 | tee "$OUT/build.log"

  [ "$mode" = "build" ] || return 0
  local vmlinux
  vmlinux="$OUT/vmlinux-${version}-${SUFFIX}"
  python3 "$KDIR/verify-pvh.py" "$vmlinux"
  echo "$VARIANT kernel: $vmlinux"
  sha256sum "$vmlinux"
}

main() {
  if [ "${1:-}" = "--inner" ]; then
    inner "${2:-build}"
    return
  fi
  case "${1:-build}" in
    build|regen-config) host "${1:-build}" ;;
    *) echo "usage: $0 large|desktop [build|regen-config]" >&2; exit 64 ;;
  esac
}

main "$@"
