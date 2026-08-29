#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

pub struct FakeRuntime {
    root: PathBuf,
    executable: PathBuf,
    log: PathBuf,
}

impl FakeRuntime {
    pub fn install() -> Self {
        let root = std::env::temp_dir().join(format!("soma-cli-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).expect("create fixture directory");
        let executable = root.join("container");
        let log = root.join("container.log");
        fs::write(&executable, SCRIPT).expect("write fake container executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .expect("make fake executable runnable");
        Self {
            root,
            executable,
            log,
        }
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn state_root(&self) -> PathBuf {
        self.root.join("soma-state")
    }

    pub fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for FakeRuntime {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const SCRIPT: &str = r#"#!/bin/sh
log="${0}.log"
state="${0}.state"
printf 'CALL\n' >> "$log"
for argument in "$@"; do
  printf '<%s>\n' "$argument" >> "$log"
done

save_resources() {
  previous=''
  saved_name=''
  saved_cpus='1'
  saved_memory='1024M'
  saved_network='default'
  for argument in "$@"; do
    case "$previous" in
      --name) saved_name="$argument" ;;
      --cpus) saved_cpus="$argument" ;;
      --memory) saved_memory="$argument" ;;
      --network) saved_network="$argument" ;;
    esac
    previous="$argument"
  done
  printf '%s %s %s %s\n' "$saved_name" "$saved_cpus" "${saved_memory%M}" "$saved_network" > "$state"
}

if [ "$1" = "system" ] && [ "$2" = "version" ]; then
  printf '[{"appName":"container","buildType":"release","commit":"fixture","version":"1.3.0"}]\n'
  exit 0
fi

if [ "$1" = "system" ] && [ "$2" = "status" ]; then
  printf '{"status":"running"}\n'
  exit 0
fi

if [ "$1" = "image" ] && [ "$2" = "pull" ]; then
  exit 0
fi

if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  printf '[{"configuration":{"descriptor":{"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}},"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","variants":[{"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","platform":{"architecture":"arm64","os":"linux","variant":"v8"}}]}]\n'
  exit 0
fi

case "$1" in
  run)
    save_resources "$@"
    case "$*" in
      *'/bin/fail'*)
        printf 'failed guest stdout\n'
        printf 'failed guest stderr\n' >&2
        exit 42
        ;;
      *'/bin/slow'*)
        sleep 2
        exit 0
        ;;
      *'/bin/noisy'*)
        printf '0123456789abcdef'
        exit 0
        ;;
      *'/bin/binary'*)
        printf '\377\000A'
        printf '\376B' >&2
        exit 0
        ;;
      *)
        printf 'v22.fixture\n'
        printf 'guest warning\n' >&2
        exit 0
        ;;
    esac
    ;;
  create)
    save_resources "$@"
    exit 0
    ;;
  start)
    case "$*" in
      *'soma-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'*) exit 9 ;;
      *) exit 0 ;;
    esac
    ;;
  exec)
    case "$*" in
      *'/bin/fail'*)
        printf 'failed guest stdout\n'
        printf 'failed guest stderr\n' >&2
        exit 42
        ;;
      *'/bin/slow'*)
        sleep 2
        exit 0
        ;;
      *'/bin/noisy'*)
        printf '0123456789abcdef'
        exit 0
        ;;
      *'/bin/binary'*)
        printf '\377\000A'
        printf '\376B' >&2
        exit 0
        ;;
      *)
        printf 'exec fixture\n'
        exit 0
        ;;
    esac
    ;;
  stop)
    exit 0
    ;;
  delete)
    case "$*" in
      *'soma-cccccccccccccccccccccccccccccccc'*) exit 9 ;;
      *) rm -f "$state"; exit 0 ;;
    esac
    ;;
  inspect)
    name="$2"
    instance="${name#soma-}"
    cpus='1'
    memory_mib='1024'
    network='default'
    if [ -f "$state" ]; then
      read -r saved_name saved_cpus saved_memory_mib saved_network < "$state"
      if [ "$saved_name" = "$name" ]; then
        cpus="$saved_cpus"
        memory_mib="$saved_memory_mib"
        network="$saved_network"
      fi
    fi
    if [ "$name" = 'soma-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' ]; then
      cpus=$((cpus + 1))
    fi
    memory_bytes=$((memory_mib * 1048576))
    if [ "$network" = 'none' ]; then
      networks='[]'
    else
      networks='[{"network":"default"}]'
    fi
    printf '[{"configuration":{"id":"%s","labels":{"io.miosa.soma.instance":"%s"},"networks":%s,"publishedPorts":[],"resources":{"cpus":%s,"memoryInBytes":%s}},"id":"%s","status":{"networks":%s,"state":"running"},"fixture":true}]\n' "$name" "$instance" "$networks" "$cpus" "$memory_bytes" "$name" "$networks"
    exit 0
    ;;
esac

exit 64
"#;
