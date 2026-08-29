//! Live `x86_64` PVH kernel-boot proof: the machine-contract acceptance test.
//!
//! Run on an `x86_64` Linux host with readable and writable `/dev/kvm` and the pinned kernel:
//!
//! ```sh
//! SOMA_X86_64_VMLINUX=/path/to/vmlinux-<ver>-soma-v1 \
//!   cargo test --locked -p soma-kvm --test x86_64_kernel_boot -- --ignored --test-threads=1
//! ```
//!
//! Without the variable the test polls the pinned-kernel output directories named in
//! `live::kernel_candidates` and fails explicitly if no stable kernel appears.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_kernel_boot/newc.rs"]
mod x86_64_newc;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_kernel_boot/discover.rs"]
mod x86_64_discover;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "x86_64_kernel_boot/host_sample.rs"]
mod x86_64_host_sample;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod live {
    use std::{
        fs,
        io::Read as _,
        path::{Path, PathBuf},
        process::Command,
        sync::{Mutex, MutexGuard, PoisonError},
        time::Duration,
    };

    use soma_kvm::x86_64::{
        BootKernelConfig, BootNonce, ElfError, GuestExit, MachineErrorKind, Phase, run_kernel_boot,
    };

    use crate::{
        x86_64_discover::{kernel_path, sha256_of},
        x86_64_host_sample::{self, Sampler},
        x86_64_newc,
    };

    const RAM_BYTES: u64 = 256 * 1024 * 1024;
    const BOOT_DEADLINE: Duration = Duration::from_secs(5);

    static LIVE_PROOF: Mutex<()> = Mutex::new(());

    fn serialize_live_proof() -> MutexGuard<'static, ()> {
        LIVE_PROOF.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn require_kvm() {
        let ok = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok();
        assert!(
            ok,
            "prerequisite failed: this live test needs a readable and writable /dev/kvm; it never passes silently"
        );
    }

    fn open_descriptor_count() -> usize {
        fs::read_dir("/proc/self/fd")
            .expect("the KVM live-test host must mount procfs")
            .count()
    }

    fn scratch_dir() -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("x86_64-kernel-boot");
        fs::create_dir_all(&dir).expect("create scratch directory under target/");
        dir
    }

    fn build_initramfs() -> PathBuf {
        let dir = scratch_dir();
        // A prebuilt archive lets the proof run where python3 or Docker is unavailable,
        // such as a container that only receives /dev/kvm.
        if let Some(prebuilt) = std::env::var_os("SOMA_X86_64_INITRAMFS") {
            let path = PathBuf::from(prebuilt);
            assert!(
                path.is_file(),
                "SOMA_X86_64_INITRAMFS must name an existing newc archive"
            );
            return path;
        }
        let init = dir.join("init");
        let _ = fs::remove_file(&init);
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/x86_64/build_x86_64_init.py");
        let status = Command::new("python3")
            .arg(&script)
            .arg(&init)
            .status()
            .expect("python3 must be available to build the init fixture");
        assert!(status.success(), "init fixture build failed");
        let init_bytes = fs::read(&init).unwrap();
        let archive = x86_64_newc::build_initramfs(&init_bytes);
        let path = dir.join("initramfs.cpio");
        fs::write(&path, archive).unwrap();
        path
    }

    fn fresh_nonce() -> BootNonce {
        let mut bytes = [0_u8; 8];
        fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .expect("read fresh nonce bytes");
        BootNonce::new(bytes)
    }

    fn print_serial(serial: &[u8], path: &Path) {
        fs::write(path, serial).unwrap();
        let text = String::from_utf8_lossy(serial);
        let lines: Vec<&str> = text.lines().collect();
        eprintln!(
            "serial log ({} bytes, {} lines) retained at {}",
            serial.len(),
            lines.len(),
            path.display()
        );
        for line in lines.iter().rev().take(12).rev() {
            eprintln!("  | {line}");
        }
    }

    #[test]
    #[ignore = "requires a Linux x86_64 runner with accessible /dev/kvm and the pinned kernel"]
    fn boots_pinned_pvh_kernel_to_challenge_bound_serial_sentinel() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let kernel = kernel_path();
        let initramfs = build_initramfs();
        let nonce = fresh_nonce();
        eprintln!(
            "kernel={} sha256={:?}",
            kernel.display(),
            sha256_of(&kernel)
        );
        let config = BootKernelConfig::new(kernel, RAM_BYTES, BOOT_DEADLINE)
            .with_initramfs(initramfs)
            .with_nonce(nonce);

        let fd_before = open_descriptor_count();
        let host_before = x86_64_host_sample::sample(RAM_BYTES / 1024);
        let sampler = Sampler::start(RAM_BYTES / 1024);
        let result = run_kernel_boot(&config);
        let (host_last, host_peak, host_peaks) = sampler.stop();
        let fd_after = open_descriptor_count();
        x86_64_host_sample::describe("host_before_run", &host_before);
        x86_64_host_sample::describe("host_last_sample_with_guest_mapped", &host_last);
        x86_64_host_sample::describe("host_peak_vmrss_while_running", &host_peak);
        eprintln!("host_peaks_while_running: {host_peaks:?}");
        let log = scratch_dir().join("serial.log");
        let evidence = match result {
            Ok(evidence) => evidence,
            Err(failure) => {
                print_serial(failure.serial(), &log);
                panic!("kernel boot failed: {failure}");
            }
        };
        print_serial(evidence.serial(), &log);
        for timing in evidence.timings() {
            eprintln!(
                "phase={:?} elapsed_ns={}",
                timing.phase(),
                timing.elapsed_ns()
            );
        }
        eprintln!(
            "cmdline={:?} entry={:#x} initramfs={:?} exit={:?} total_ns={} bus={:?} uart={:?} fd_before={fd_before} fd_after={fd_after}",
            evidence.cmdline(),
            evidence.entry(),
            evidence.initramfs(),
            evidence.exit(),
            evidence.total_ns(),
            evidence.bus_counters(),
            evidence.serial_counters()
        );

        let expected = nonce.sentinel();
        let text = String::from_utf8_lossy(evidence.serial());
        assert!(
            text.lines()
                .any(|line| line.trim_end_matches('\r') == expected),
            "sentinel line {expected:?} missing from serial output"
        );
        assert!(
            matches!(
                evidence.exit(),
                GuestExit::Halt | GuestExit::Shutdown | GuestExit::Reset
            ),
            "unexpected exit {:?}",
            evidence.exit()
        );
        assert_eq!(fd_after, fd_before, "the proof leaked file descriptors");
        let run_ns = evidence
            .timings()
            .iter()
            .find(|t| t.phase() == Phase::Run)
            .map(|t| t.elapsed_ns())
            .unwrap();
        assert!(
            u128::from(run_ns) <= BOOT_DEADLINE.as_nanos(),
            "boot exceeded the deadline"
        );
    }

    #[test]
    #[ignore = "requires a Linux x86_64 runner with accessible /dev/kvm and the pinned kernel"]
    fn corrupted_pvh_note_is_rejected_before_kvm_run() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let kernel = kernel_path();
        let mut image = fs::read(&kernel).unwrap();
        let header: Vec<u8> = [4_u32, 8, 18]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .chain(*b"Xen\0")
            .collect();
        let position = image
            .windows(header.len())
            .position(|window| window == header.as_slice())
            .expect("the pinned kernel carries an eight-byte XEN_ELFNOTE_PHYS32_ENTRY note");
        image[position + 12] = b'Z';
        let corrupted = scratch_dir().join("vmlinux-corrupted-note");
        fs::write(&corrupted, image).unwrap();

        let fd_before = open_descriptor_count();
        let failure = run_kernel_boot(&BootKernelConfig::new(corrupted, RAM_BYTES, BOOT_DEADLINE))
            .expect_err("a kernel without the PVH note must be rejected");
        let fd_after = open_descriptor_count();
        eprintln!("error={failure} fd_before={fd_before} fd_after={fd_after}");
        assert_eq!(failure.error().phase(), Phase::LoadGuest);
        assert_eq!(
            failure.error().kind(),
            &MachineErrorKind::Elf(ElfError::MissingPvhNote)
        );
        assert!(failure.serial().is_empty());
        assert_eq!(fd_after, fd_before);
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn reports_that_the_x86_64_kernel_boot_is_unavailable_on_this_target() {
    // The x86_64 machine is compiled only for Linux x86_64 and is never emulated elsewhere.
    assert!(std::hint::black_box(true));
}
