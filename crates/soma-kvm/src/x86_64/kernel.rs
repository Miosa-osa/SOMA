//! The PVH cold-boot proof: the pinned kernel to a challenge-bound serial sentinel.

mod config;

use std::{fs, io::Read as _};

use vmm_sys_util::eventfd::EventFd;

pub use self::config::BootKernelConfig;
use super::{
    InterruptController, Machine, MachineError, Phase,
    loader::{self, INITRAMFS_LIMIT, KERNEL_IMAGE_LIMIT, LoadedKernel},
    ports::{BusCounters, PortBus},
    run::GuestExit,
    serial::{SERIAL_GSI, Serial, SerialCounters},
    timing::{PhaseTiming, Stopwatch},
    watchdog,
};

/// Retained evidence from a successful boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelBootEvidence {
    serial: Vec<u8>,
    exit: GuestExit,
    timings: Vec<PhaseTiming>,
    total_ns: u64,
    cmdline: String,
    entry: u64,
    initramfs: Option<(u64, u64)>,
    bus: BusCounters,
    uart: SerialCounters,
}

impl KernelBootEvidence {
    /// Every byte the guest wrote to the serial port, in order.
    #[must_use]
    pub fn serial(&self) -> &[u8] {
        &self.serial
    }

    /// How the guest stopped.
    #[must_use]
    pub const fn exit(&self) -> GuestExit {
        self.exit
    }

    /// Per-phase durations in lifecycle order, ending with cleanup.
    #[must_use]
    pub fn timings(&self) -> &[PhaseTiming] {
        &self.timings
    }

    /// Nanoseconds from reading the kernel through completed cleanup.
    #[must_use]
    pub const fn total_ns(&self) -> u64 {
        self.total_ns
    }

    /// The exact command line written to the guest.
    #[must_use]
    pub fn cmdline(&self) -> &str {
        &self.cmdline
    }

    /// The validated PVH entry point.
    #[must_use]
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    /// Guest-physical start and size of the initramfs when one was loaded.
    #[must_use]
    pub const fn initramfs(&self) -> Option<(u64, u64)> {
        self.initramfs
    }

    /// Port-access counts by device.
    #[must_use]
    pub const fn bus_counters(&self) -> BusCounters {
        self.bus
    }

    /// Register-access counts inside the 16550 model.
    #[must_use]
    pub const fn serial_counters(&self) -> SerialCounters {
        self.uart
    }
}

/// A failed boot together with whatever console output the guest produced first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelBootFailure {
    error: MachineError,
    serial: Vec<u8>,
}

impl KernelBootFailure {
    /// The typed failure.
    #[must_use]
    pub const fn error(&self) -> &MachineError {
        &self.error
    }

    /// Console bytes captured before the failure, possibly empty.
    #[must_use]
    pub fn serial(&self) -> &[u8] {
        &self.serial
    }
}

impl std::fmt::Display for KernelBootFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

impl std::error::Error for KernelBootFailure {}

/// Boots the pinned PVH kernel once on one vCPU and releases everything it owned.
///
/// # Errors
///
/// Returns the typed phase failure, with any captured console bytes, when an artifact cannot be
/// read or validated, when the host lacks KVM or a capability, when the guest exits in an
/// unexpected way, or when it does not stop before the deadline.
pub fn run_kernel_boot(config: &BootKernelConfig) -> Result<KernelBootEvidence, KernelBootFailure> {
    let mut clock = Stopwatch::new();
    let outcome = boot(config, &mut clock);
    clock.lap(Phase::Cleanup);
    let (total_ns, timings) = clock.finish();
    match outcome {
        Ok((serial, exit, loaded, bus, uart)) => Ok(KernelBootEvidence {
            serial,
            exit,
            timings,
            total_ns,
            cmdline: loaded.cmdline,
            entry: loaded.entry,
            initramfs: loaded.initramfs,
            bus,
            uart,
        }),
        Err((error, serial)) => Err(KernelBootFailure { error, serial }),
    }
}

type BootOutcome = (
    Vec<u8>,
    GuestExit,
    LoadedKernel,
    BusCounters,
    SerialCounters,
);

fn boot(
    config: &BootKernelConfig,
    clock: &mut Stopwatch,
) -> Result<BootOutcome, (MachineError, Vec<u8>)> {
    let image = read_bounded(&config.kernel, KERNEL_IMAGE_LIMIT).map_err(|e| (e, Vec::new()))?;
    let initramfs = match &config.initramfs {
        Some(path) => Some(read_bounded(path, INITRAMFS_LIMIT).map_err(|e| (e, Vec::new()))?),
        None => None,
    };
    clock.lap(Phase::ReadKernel);
    let mut machine = Machine::create(config.ram_bytes, clock).map_err(|e| (e, Vec::new()))?;
    let outcome = prepare_and_run(&mut machine, config, &image, initramfs.as_deref(), clock);
    drop(machine);
    outcome
}

fn prepare_and_run(
    machine: &mut Machine,
    config: &BootKernelConfig,
    image: &[u8],
    initramfs: Option<&[u8]>,
    clock: &mut Stopwatch,
) -> Result<BootOutcome, (MachineError, Vec<u8>)> {
    let prepared = (|| {
        machine.configure_platform(InterruptController::InKernel, config.pit, clock)?;
        let loaded =
            loader::load_kernel(&mut machine.ram, image, initramfs, config.nonce.as_ref())?;
        clock.lap(Phase::LoadGuest);
        let vcpu = machine.boot_vcpu(loaded.entry, clock)?;
        let line = EventFd::new(libc::EFD_NONBLOCK)
            .map_err(|error| MachineError::io(Phase::Run, &error))?;
        machine
            .vm
            .register_irqfd(&line, SERIAL_GSI)
            .map_err(|error| MachineError::os(Phase::Run, error))?;
        Ok((loaded, vcpu, line))
    })();
    let (loaded, vcpu, line) = prepared.map_err(|error| (error, Vec::new()))?;
    let bus = PortBus::new(Serial::new(Some(line)));
    let sentinel = config
        .nonce
        .filter(|_| config.stop_on_sentinel)
        .map(|nonce| nonce.sentinel().into_bytes());
    let report = watchdog::run_with_deadline(vcpu, bus, sentinel, config.timeout);
    clock.lap(Phase::Run);
    let (serial, bus, uart) = report.bus.map_or(
        (
            Vec::new(),
            BusCounters::default(),
            SerialCounters::default(),
        ),
        |bus| {
            (
                bus.serial().output().to_vec(),
                bus.counters(),
                bus.serial_counters(),
            )
        },
    );
    match report.result {
        Ok(exit) => Ok((serial, exit, loaded, bus, uart)),
        Err(error) => Err((error, serial)),
    }
}

fn read_bounded(path: &std::path::Path, limit: u64) -> Result<Vec<u8>, MachineError> {
    let file = fs::File::open(path).map_err(|error| MachineError::io(Phase::ReadKernel, &error))?;
    let length = file
        .metadata()
        .map_err(|error| MachineError::io(Phase::ReadKernel, &error))?
        .len();
    if length == 0 || length > limit {
        return Err(MachineError::invalid(
            Phase::ReadKernel,
            "artifact is empty or exceeds its size bound",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| MachineError::io(Phase::ReadKernel, &error))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::Duration};

    use super::*;
    use crate::x86_64::MachineErrorKind;

    #[test]
    fn missing_kernel_fails_in_the_read_phase_before_kvm() {
        let config = BootKernelConfig::new(
            PathBuf::from("/nonexistent/soma-vmlinux"),
            128 * 1024 * 1024,
            Duration::from_secs(1),
        );
        let failure = run_kernel_boot(&config).unwrap_err();
        assert_eq!(failure.error().phase(), Phase::ReadKernel);
        assert!(matches!(failure.error().kind(), MachineErrorKind::Os(_)));
        assert!(failure.serial().is_empty());
    }
}
