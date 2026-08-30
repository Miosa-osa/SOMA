//! Live `x86_64` sandbox proof: a compiled Generation cold-boots on KVM, the static guest agent
//! consumes the launch page, authenticates over vsock, answers the readiness probe, executes
//! one bounded command, and shuts down through the authenticated channel.
//!
//! Run on an `x86_64` Linux host with readable and writable `/dev/kvm`, the pinned kernel, the
//! pinned erofs-utils, the built static guest agent, and Docker for the image export:
//!
//! ```sh
//! SOMA_X86_64_VMLINUX=/path/to/vmlinux-<ver>-soma-v1 \
//! SOMA_EROFS_TOOLS=/path/to/erofs-utils-1.9.4 \
//! SOMA_GUEST_AGENT=target/x86_64-unknown-linux-musl/release/soma-guest-agent \
//!   cargo test --locked -p soma-kvm --test x86_64_sandbox_boot -- --ignored --test-threads=1
//! ```

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_kernel_boot/discover.rs"]
mod x86_64_discover;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_kernel_boot/host_sample.rs"]
mod x86_64_host_sample;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_sandbox_boot/generation.rs"]
mod x86_64_sandbox_boot_generation;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_sandbox_boot/generation_cache.rs"]
mod x86_64_sandbox_boot_generation_cache;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_sandbox_boot/control.rs"]
mod x86_64_sandbox_boot_control;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_sandbox_boot/session.rs"]
mod x86_64_sandbox_boot_session;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_sandbox_boot/host.rs"]
mod x86_64_sandbox_boot_host;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod live {
    use std::{
        fs,
        path::Path,
        sync::{Mutex, MutexGuard, PoisonError},
    };

    use soma_generation::open_artifact;

    use crate::{
        x86_64_discover::{kernel_path, sha256_of},
        x86_64_host_sample::{self, Sampler},
        x86_64_sandbox_boot_generation as generation,
        x86_64_sandbox_boot_host::{
            Proof, assert_proof, open_descriptor_count, require_kvm, require_scratch_space,
            scratch_dir, thread_count,
        },
        x86_64_sandbox_boot_session as session,
    };

    const MIB: u64 = 1024 * 1024;
    const BUSYBOX: &str = "busybox:stable-musl";
    const NODE: &str = "node:22";
    const MAC_NODE_TREE_DIGEST: &str =
        "sha256:5dac6c571b970375a978c3f2f8777883e5bdd582fb4b43a5b872f929a2c7adf6";

    static LIVE_PROOF: Mutex<()> = Mutex::new(());

    fn serialize_live_proof() -> MutexGuard<'static, ()> {
        LIVE_PROOF.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[allow(clippy::too_many_lines)]
    fn boot_generation(
        name: &str,
        image: &str,
        override_var: &str,
        memory_mib: u64,
        storage_mib: u64,
        command: &session::Command<'_>,
    ) -> Option<Proof> {
        require_scratch_space();
        let scratch = scratch_dir(name);
        let kernel = kernel_path();
        eprintln!(
            "kernel={} sha256={:?}",
            kernel.display(),
            sha256_of(&kernel)
        );
        let inputs = generation::inputs(kernel);
        let layout = generation::oci_layout(image, override_var, &scratch)?;
        let compiled = generation::compile(
            &layout,
            &format!("docker.io/library/{image}"),
            generation::Shape {
                memory_mib,
                storage_mib,
            },
            &inputs,
            &scratch,
        );
        let manifest = &compiled.manifest();
        eprintln!(
            "[{name}] generation_id={} tree={} entries={} root={} ({} bytes) initramfs={} agent={} kernel={} overlay_template={} ({} bytes)",
            compiled.id().as_str(),
            compiled.tree_digest.as_str(),
            compiled.entry_count,
            manifest.root.descriptor.digest,
            manifest.root.descriptor.size,
            manifest.initramfs.descriptor.digest,
            manifest.guest_agent.descriptor.digest,
            manifest.kernel.descriptor.digest,
            manifest.overlay.templates[0].descriptor.digest,
            manifest.overlay.templates[0].descriptor.size,
        );
        // Every descriptor opened from here on is handed to the machine or dropped before the
        // matching count, so equality proves the sandbox released everything it received.
        let threads_before = thread_count();
        let fd_before = open_descriptor_count();
        let kernel = open_artifact(&compiled.store, &manifest.kernel.descriptor).unwrap();
        let initramfs = open_artifact(&compiled.store, &manifest.initramfs.descriptor).unwrap();
        let root = open_artifact(&compiled.store, &manifest.root.descriptor).unwrap();
        let mut template =
            open_artifact(&compiled.store, &manifest.overlay.templates[0].descriptor).unwrap();
        let head_path = scratch.join("overlay-head.ext4");
        let head = generation::private_head(&mut template, &head_path);
        drop(template);
        let root_before = generation::sha256_file(&root);
        let head_before = generation::sha256_file(&head);
        assert_eq!(
            format!("sha256:{head_before}"),
            manifest.overlay.templates[0].descriptor.digest.to_string(),
            "the private head must start as an exact copy of the sterile template"
        );
        let ram_bytes = memory_mib * MIB;
        let config = session::config(kernel, initramfs, root, head, ram_bytes);
        let expected_cmdline = String::from_utf8(manifest.command_line.clone()).unwrap();

        let host_before = x86_64_host_sample::sample(ram_bytes / 1024);
        let sampler = Sampler::start(ram_bytes / 1024);
        let (evidence, outcome) = session::run(config, compiled.id().as_str(), command);
        let (host_last, host_peak, host_peaks) = sampler.stop();
        let fd_after = open_descriptor_count();
        let threads_after = thread_count();
        x86_64_host_sample::describe("host_before_run", &host_before);
        x86_64_host_sample::describe("host_last_sample_with_guest_mapped", &host_last);
        x86_64_host_sample::describe("host_peak_vmrss_while_running", &host_peak);
        eprintln!("host_peaks_while_running: {host_peaks:?}");
        session::report(name, &evidence, &scratch.join("serial.log"));
        eprintln!(
            "[{name}] fd_before={fd_before} fd_after={fd_after} threads_before={threads_before} threads_after={threads_after}"
        );
        let (hostile, executed) = match outcome {
            Ok(executed) => executed,
            Err(error) => panic!("[{name}] session failed: {error}; exit={:?}", evidence.exit),
        };
        eprintln!(
            "[{name}] hostile status={:?} stdout={} stderr={} bytes",
            hostile.status,
            hostile.stdout.len(),
            hostile.stderr.len()
        );
        eprintln!(
            "[{name}] command status={:?} stdout={:?} stderr={:?}",
            executed.status,
            String::from_utf8_lossy(&executed.stdout),
            String::from_utf8_lossy(&executed.stderr)
        );
        assert_eq!(
            evidence.cmdline, expected_cmdline,
            "machine and manifest disagree"
        );
        let root = open_artifact(&compiled.store, &manifest.root.descriptor).unwrap();
        let head = fs::File::open(&head_path).unwrap();
        Some(Proof {
            root_after: generation::sha256_file(&root),
            head_after: generation::sha256_file(&head),
            evidence,
            hostile,
            executed,
            fd_before,
            fd_after,
            threads_before,
            threads_after,
            root_before,
            head_before,
        })
    }

    #[test]
    #[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker"]
    fn tiny_generation_boots_authenticates_and_executes_one_command() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let command = session::Command {
            program: b"/bin/busybox",
            arguments: &[b"uname", b"-a"],
            timeout_millis: 10_000,
            output_bytes: 65_536,
        };
        let proof = boot_generation(
            "busybox",
            BUSYBOX,
            "SOMA_OCI_BUSYBOX_LAYOUT",
            256,
            64,
            &command,
        )
        .expect("prerequisite failed: the busybox OCI layout could not be exported; install Docker or set SOMA_OCI_BUSYBOX_LAYOUT");
        assert_proof(&proof);
        let stdout = String::from_utf8_lossy(&proof.executed.stdout);
        assert!(stdout.starts_with("Linux soma-"), "stdout={stdout:?}");
        assert!(stdout.contains("6.12.107-soma-v1"));
        assert!(stdout.contains("x86_64"));
        assert!(proof.executed.stderr.is_empty());
    }

    #[test]
    #[ignore = "requires /dev/kvm, the pinned kernel, erofs-utils, the static guest agent, and Docker with node:22"]
    fn node_22_generation_boots_authenticates_and_reports_its_version() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let command = session::Command {
            program: b"/usr/local/bin/node",
            arguments: &[b"--version"],
            timeout_millis: 30_000,
            output_bytes: 65_536,
        };
        let proof = boot_generation("node22", NODE, "SOMA_OCI_NODE_LAYOUT", 1024, 1024, &command)
            .expect("prerequisite failed: the node:22 OCI layout could not be exported; set SOMA_OCI_NODE_LAYOUT");
        assert_proof(&proof);
        let stdout = String::from_utf8_lossy(&proof.executed.stdout);
        assert!(stdout.starts_with("v22."), "stdout={stdout:?}");
        let _ = Path::new(MAC_NODE_TREE_DIGEST);
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn reports_that_the_x86_64_sandbox_boot_is_unavailable_on_this_target() {
    // The x86_64 sandbox machine is compiled only for Linux x86_64 and is never emulated elsewhere.
    assert!(std::hint::black_box(true));
}
