//! Live `x86_64` KVM halt-guest proof.
//!
//! Run on an `x86_64` Linux host with readable and writable `/dev/kvm`:
//!
//! ```sh
//! cargo test --locked -p soma-kvm --test x86_64_halt_guest -- --ignored --nocapture
//! ```

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod live {
    use std::{
        fs,
        sync::{Mutex, MutexGuard, PoisonError},
        time::Duration,
    };

    use soma_kvm::x86_64::{
        EXPECTED_SERIAL, GuestExit, HaltGuestConfig, InterruptController, MachineErrorKind, Phase,
        run_halt_guest,
    };

    const RAM_BYTES: u64 = 128 * 1024 * 1024;

    /// Descriptor accounting is process-wide, so the live proofs never overlap in one process.
    static LIVE_PROOF: Mutex<()> = Mutex::new(());

    fn serialize_live_proof() -> MutexGuard<'static, ()> {
        LIVE_PROOF.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn require_kvm() {
        let readable_writable = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok();
        assert!(
            readable_writable,
            "prerequisite failed: this live test needs a readable and writable /dev/kvm; it never passes silently"
        );
    }

    fn open_descriptor_count() -> usize {
        fs::read_dir("/proc/self/fd")
            .expect("the KVM live-test host must mount procfs")
            .count()
    }

    #[test]
    #[ignore = "requires a Linux x86_64 runner with accessible /dev/kvm"]
    fn halts_after_writing_soma_to_the_serial_port_and_releases_descriptors() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let fd_before = open_descriptor_count();
        let evidence = run_halt_guest(&HaltGuestConfig::new(RAM_BYTES, Duration::from_secs(10)))
            .expect("the x86_64 halt guest must run to KVM_EXIT_HLT");
        let fd_after = open_descriptor_count();

        for timing in evidence.timings() {
            println!(
                "phase={:?} elapsed_ns={}",
                timing.phase(),
                timing.elapsed_ns()
            );
        }
        println!(
            "serial={:?} exit={:?} total_ns={} fd_before={fd_before} fd_after={fd_after}",
            String::from_utf8_lossy(evidence.serial()),
            evidence.exit(),
            evidence.total_ns()
        );

        assert_eq!(evidence.serial(), EXPECTED_SERIAL);
        assert_eq!(evidence.exit(), GuestExit::Halt);
        assert_eq!(fd_after, fd_before, "the proof leaked file descriptors");
        let phases: Vec<Phase> = evidence.timings().iter().map(|t| t.phase()).collect();
        assert_eq!(phases.first(), Some(&Phase::Open));
        assert_eq!(phases.last(), Some(&Phase::Cleanup));
        assert!(phases.contains(&Phase::Run));
    }

    #[test]
    #[ignore = "requires a Linux x86_64 runner with accessible /dev/kvm"]
    fn in_kernel_irqchip_parks_hlt_and_the_watchdog_reclaims_the_vcpu() {
        let _serialized = serialize_live_proof();
        require_kvm();
        let fd_before = open_descriptor_count();
        let config = HaltGuestConfig::new(RAM_BYTES, Duration::from_secs(2))
            .with_interrupt_controller(InterruptController::InKernel);
        let error = run_halt_guest(&config)
            .expect_err("hlt under the in-kernel irqchip must block until the watchdog fires");
        let fd_after = open_descriptor_count();

        println!("error={error} fd_before={fd_before} fd_after={fd_after}");
        assert_eq!(error.phase(), Phase::Run);
        assert_eq!(error.kind(), &MachineErrorKind::Timeout);
        assert_eq!(
            fd_after, fd_before,
            "the timed-out proof leaked file descriptors"
        );
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn reports_that_the_x86_64_halt_guest_is_unavailable_on_this_target() {
    // The x86_64 machine floor is compiled only for Linux x86_64 and is never emulated elsewhere.
    assert!(std::hint::black_box(true));
}
