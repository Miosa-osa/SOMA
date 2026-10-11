//! Early-init support for a container engine inside the guest.
//!
//! The guest has no init system, so nothing starts `dockerd` at boot and nothing lays out what it
//! needs. The agent does the two parts that only PID 1 can do before the root is entered:
//!
//! * mount the cgroup v2 hierarchy at `/sys/fs/cgroup` and delegate the controllers a container
//!   runtime places limits with, and
//! * bind the engine's data root onto the private ext4 head. `overlay2` refuses to sit on an
//!   `OverlayFS`, and the composed root is one, so `/var/lib/docker` has to be a directory of the
//!   head itself, not a path inside the overlay.
//!
//! Starting the engine is a lazy, on-demand step in the image (`soma-docker-ensure`), not boot
//! work, so a guest that never touches Docker pays for nothing beyond these two mounts.
//!
//! None of this can fail the boot. A machine whose image has an engine but whose head cannot hold
//! its data root still boots and still runs every other command; the engine then reports its own
//! error the first time something asks for it, which is the failure the user can act on.

use std::fs;
use std::path::Path;

use crate::mounts;

/// Where the engine's data lives on the head, beside `upper` and `work`.
pub(super) const DATA_DIR_NAME: &str = "docker";
/// The cgroup v2 mount point.
const CGROUP_MOUNT: &str = "/sys/fs/cgroup";
/// The controllers a container runtime places limits with: CPU shares and bandwidth, memory,
/// block IO, pid counts, and cpuset. A controller the kernel lacks makes its own write fail and
/// the others still apply, so each is written separately.
const CONTROLLERS: [&str; 5] = ["cpu", "memory", "io", "pids", "cpuset"];

/// True when the immutable lower carries a container engine.
pub(super) fn image_has_engine(lower: &str) -> bool {
    Path::new(lower).join("usr/bin/dockerd").is_file()
}

/// Mounts cgroup v2 and delegates the container controllers from the root cgroup.
pub(super) fn mount_cgroups() {
    // A failure here (no cgroup support, already mounted) leaves the engine to report its own
    // error; see the module note on why it never fails the boot.
    if mounts::mount(
        "cgroup2",
        CGROUP_MOUNT,
        "cgroup2",
        libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC,
        "",
    )
    .is_err()
    {
        return;
    }
    for controller in CONTROLLERS {
        let _ = fs::write(
            format!("{CGROUP_MOUNT}/cgroup.subtree_control"),
            format!("+{controller}"),
        );
    }
}

/// Bind-mounts `<head>/docker` onto `<root>/var/lib/docker`.
///
/// The bind is a child of the composed root, so the move that makes the root `/` carries it.
pub(super) fn bind_data_root(head: &str, root: &str) {
    let source = format!("{head}/{DATA_DIR_NAME}");
    let target = format!("{root}/var/lib/docker");
    if fs::create_dir_all(&source).is_err() || fs::create_dir_all(&target).is_err() {
        return;
    }
    let _ = mounts::bind(&source, &target);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_has_an_engine_only_when_dockerd_is_a_file_in_the_lower() {
        let dir = std::env::temp_dir().join(format!("soma-engine-probe-{}", std::process::id()));
        let lower = dir.to_str().expect("utf8 temp path");
        assert!(!image_has_engine(lower));
        fs::create_dir_all(dir.join("usr/bin")).expect("tree");
        assert!(!image_has_engine(lower));
        fs::write(dir.join("usr/bin/dockerd"), b"").expect("file");
        assert!(image_has_engine(lower));
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn the_data_root_lives_beside_upper_and_work_on_the_head() {
        assert_eq!(DATA_DIR_NAME, "docker");
        assert!(!["lost+found", "upper", "work"].contains(&DATA_DIR_NAME));
    }

    #[test]
    fn unprivileged_setup_never_panics() {
        mount_cgroups();
        bind_data_root("/nonexistent-head", "/nonexistent-root");
    }
}
