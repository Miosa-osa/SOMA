#!/usr/bin/env bash
# Reproducible build of the SOMA x86_64 machine contract v1 guest kernel.
#
# Usage (from the repository root or the kernel directory):
#   kernel/build.sh                 build vmlinux inside the pinned Docker toolchain
#   kernel/build.sh regen-config    regenerate config-x86_64-soma-v1 from soma-v1.fragment
#   kernel/build.sh --inner ...     internal entry point executed inside the container
#
# Inputs:  source.json, config-x86_64-soma-v1, required-config.txt, Dockerfile
# Outputs: out/vmlinux-<version>-soma-v1, out/vmlinux-<version>-soma-v1.sha256,
#          out/manifest.json, out/build.log
#
# Every input is pinned: the source tarball by SHA-256, the toolchain by base
# image digest plus verified gcc/ld/make versions, and the build metadata by
# fixed KBUILD_* and SOURCE_DATE_EPOCH values.  Any drift fails closed.

set -euo pipefail

KDIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$KDIR/out"

json_field() {
  python3 - "$1" "$2" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
for part in sys.argv[2].split("."):
    value = value[part]
print(value)
PY
}

sha256_of() { sha256sum "$1" | awk '{print $1}'; }

# --------------------------------------------------------------------------
# Inner phase: runs inside the builder container as the invoking user.
# --------------------------------------------------------------------------
inner() {
  local mode="$1"
  local version tarball srcdir cfg jobs
  version="$(json_field /work/source.json kernel_version)"
  tarball="/work/out/src/linux-${version}.tar.xz"
  srcdir="/work/out/build/linux-${version}"
  cfg="/work/$(json_field /work/source.json config)"
  jobs="${SOMA_KERNEL_JOBS:-$(nproc)}"

  local want_gcc want_ld want_make
  want_gcc="$(json_field /work/source.json toolchain.gcc)"
  want_ld="$(json_field /work/source.json toolchain.ld)"
  want_make="$(json_field /work/source.json toolchain.make)"
  [ "$(gcc -dumpfullversion)" = "$want_gcc" ] || { echo "toolchain drift: gcc $(gcc -dumpfullversion) != $want_gcc" >&2; exit 2; }
  ld --version | head -1 | grep -Fq " $want_ld" || { echo "toolchain drift: $(ld --version | head -1) != $want_ld" >&2; exit 2; }
  [ "$(make --version | head -1 | awk '{print $3}')" = "$want_make" ] || { echo "toolchain drift: make != $want_make" >&2; exit 2; }
  echo "toolchain: gcc $want_gcc, $(ld --version | head -1), GNU Make $want_make"

  rm -rf /work/out/build
  mkdir -p /work/out/build
  tar -C /work/out/build -xJf "$tarball"
  cd "$srcdir"

  if [ "$mode" = "regen-config" ]; then
    make -s x86_64_defconfig
    scripts/kconfig/merge_config.sh -m .config /work/soma-v1.fragment >/dev/null
    make -s olddefconfig
    cp .config "$cfg"
    echo "regenerated $(basename "$cfg") sha256 $(sha256_of "$cfg")"
    return 0
  fi

  cp "$cfg" .config
  make -s olddefconfig
  python3 /work/verify-config.py --pinned "$cfg" --final .config --required /work/required-config.txt
  cp .config /work/out/final.config

  export KBUILD_BUILD_TIMESTAMP KBUILD_BUILD_USER KBUILD_BUILD_HOST KBUILD_BUILD_VERSION SOURCE_DATE_EPOCH
  local start end
  start="$(date +%s.%N)"
  make -j"$jobs" vmlinux
  end="$(date +%s.%N)"
  awk -v s="$start" -v e="$end" -v j="$jobs" 'BEGIN{printf "make vmlinux wall seconds: %.1f (jobs=%d)\n", e-s, j}'

  cp vmlinux "/work/out/vmlinux-${version}-soma-v1"
  ( cd /work/out && sha256sum "vmlinux-${version}-soma-v1" > "vmlinux-${version}-soma-v1.sha256" )
  echo "$jobs" > /work/out/jobs.txt
  awk -v s="$start" -v e="$end" 'BEGIN{printf "%.3f\n", e-s}' > /work/out/make-seconds.txt
}

# --------------------------------------------------------------------------
# Host phase.
# --------------------------------------------------------------------------
host() {
  local mode="$1"
  local version url want_sha tarball base_digest image_id
  version="$(json_field "$KDIR/source.json" kernel_version)"
  url="$(json_field "$KDIR/source.json" tarball_url)"
  want_sha="$(json_field "$KDIR/source.json" tarball_sha256)"
  base_digest="$(json_field "$KDIR/source.json" builder_base_image)"
  mkdir -p "$OUT/src"
  tarball="$OUT/src/linux-${version}.tar.xz"

  if [ ! -f "$tarball" ] || [ "$(sha256_of "$tarball")" != "$want_sha" ]; then
    echo "downloading $url"
    rm -f "$tarball"
    curl -fsSL --retry 3 -o "$tarball.part" "$url"
    mv "$tarball.part" "$tarball"
  fi
  local got_sha
  got_sha="$(sha256_of "$tarball")"
  [ "$got_sha" = "$want_sha" ] || { echo "source tarball sha256 mismatch: $got_sha != $want_sha" >&2; rm -f "$tarball"; exit 2; }
  echo "source: linux-${version}.tar.xz sha256 $got_sha (verified)"

  grep -Fq "FROM $base_digest" "$KDIR/Dockerfile" || { echo "Dockerfile base image does not match source.json" >&2; exit 2; }
  image_id="$(docker build -q -f "$KDIR/Dockerfile" "$KDIR")"
  echo "builder image: $image_id (base $base_digest)"

  local env_ts env_user env_host env_ver env_epoch
  env_ts="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_TIMESTAMP)"
  env_user="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_USER)"
  env_host="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_HOST)"
  env_ver="$(json_field "$KDIR/source.json" reproducible_env.KBUILD_BUILD_VERSION)"
  env_epoch="$(json_field "$KDIR/source.json" reproducible_env.SOURCE_DATE_EPOCH)"

  local start end
  start="$(date +%s)"
  docker run --rm --network none \
    -u "$(id -u):$(id -g)" \
    -v "$KDIR:/work" -w /work \
    -e KBUILD_BUILD_TIMESTAMP="$env_ts" \
    -e KBUILD_BUILD_USER="$env_user" \
    -e KBUILD_BUILD_HOST="$env_host" \
    -e KBUILD_BUILD_VERSION="$env_ver" \
    -e SOURCE_DATE_EPOCH="$env_epoch" \
    -e SOMA_KERNEL_JOBS="${SOMA_KERNEL_JOBS:-}" \
    "$image_id" bash /work/build.sh --inner "$mode" 2>&1 | tee "$OUT/build.log"
  end="$(date +%s)"
  [ "${PIPESTATUS[0]}" -eq 0 ] || { echo "container build failed" >&2; exit 2; }
  [ "$mode" = "build" ] || return 0

  local vmlinux pvh_json
  vmlinux="$OUT/vmlinux-${version}-soma-v1"
  pvh_json="$(python3 "$KDIR/verify-pvh.py" "$vmlinux" --json)" || { echo "$pvh_json"; echo "PVH verification failed" >&2; exit 2; }
  python3 "$KDIR/verify-pvh.py" "$vmlinux"

  local pkgs
  pkgs="$(docker run --rm "$image_id" dpkg-query -W -f='${Package}=${Version}\n' gcc-13 binutils make flex bison libelf-dev libssl-dev | tr '\n' ' ')"

  python3 - "$KDIR" "$OUT" "$version" "$image_id" "$pkgs" "$((end - start))" "$pvh_json" <<'PY'
import hashlib, json, os, sys
kdir, out, version, image_id, pkgs, wall, pvh_json = sys.argv[1:8]
src = json.load(open(os.path.join(kdir, "source.json"), encoding="utf-8"))
def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()
vmlinux = os.path.join(out, f"vmlinux-{version}-soma-v1")
manifest = {
    "schema_version": 1,
    "artifact": src["artifact"],
    "machine_contract": src["machine_contract"],
    "arch": src["arch"],
    "kernel_version": version,
    "source": {
        "tag": src["source_tag"],
        "tarball_url": src["tarball_url"],
        "tarball_sha256": src["tarball_sha256"],
    },
    "config": {
        "file": src["config"],
        "sha256": sha(os.path.join(kdir, src["config"])),
        "final_config_sha256": sha(os.path.join(out, "final.config")),
        "fragment_sha256": sha(os.path.join(kdir, "soma-v1.fragment")),
        "required_sha256": sha(os.path.join(kdir, "required-config.txt")),
    },
    "builder": {
        "base_image": src["builder_base_image"],
        "image_id": image_id,
        "dockerfile_sha256": sha(os.path.join(kdir, "Dockerfile")),
        "toolchain": src["toolchain"],
        "packages": pkgs.split(),
    },
    "build": {
        "env": src["reproducible_env"],
        "jobs": int(open(os.path.join(out, "jobs.txt")).read().strip()),
        "make_vmlinux_seconds": float(open(os.path.join(out, "make-seconds.txt")).read().strip()),
        "container_wall_seconds": int(wall),
        "command": "kernel/build.sh",
    },
    "output": {
        "file": f"kernel/out/vmlinux-{version}-soma-v1",
        "sha256": sha(vmlinux),
        "size_bytes": os.path.getsize(vmlinux),
    },
    "pvh": json.loads(pvh_json),
}
path = os.path.join(out, "manifest.json")
with open(path, "w", encoding="utf-8") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
print(f"manifest: {path}")
print(f"vmlinux:  {vmlinux}")
print(f"sha256:   {manifest['output']['sha256']}  size {manifest['output']['size_bytes']} bytes")
PY
}

main() {
  if [ "${1:-}" = "--inner" ]; then
    inner "${2:-build}"
    return
  fi
  local mode="${1:-build}"
  case "$mode" in
    build|regen-config) host "$mode" ;;
    *) echo "usage: $0 [build|regen-config]" >&2; exit 64 ;;
  esac
}

main "$@"
