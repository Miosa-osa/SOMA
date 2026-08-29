use std::time::Duration;

use kvm_ioctls::{DeviceFd, VcpuExit, VcpuFd, VmFd};

use super::{
    ARM64_BOOT_SENTINEL, Arm64BootError,
    command::{Arm64Command, Arm64CommandOutcome, Arm64Fixtures, PreparedCommand},
    control_uart::{ControlUart, VmIrqTrigger},
    layout::{CONTROL_UART_BASE, UART_BASE, UART_SIZE},
    machine::{self, DeviceProfile},
    response::{ResponseCollector, validate_hello},
    uart::Uart,
    watchdog::{self, TaskOutcome},
};

const BOOT_HANDSHAKE_ALLOWANCE: Duration = Duration::from_secs(10);
const TERMINAL_GRACE: Duration = Duration::from_secs(2);

pub(crate) fn execute_arm64_fixture(
    fixtures: Arm64Fixtures<'_>,
    command: Arm64Command<'_>,
) -> Result<Arm64CommandOutcome, Arm64BootError> {
    let prepared = super::command::prepare(command)?;
    let machine = machine::prepare(fixtures.kernel, fixtures.initramfs, DeviceProfile::Command)?;
    let machine::Machine {
        vcpu,
        gic,
        vm,
        memory,
    } = machine;
    let result = watchdog::run_task(vcpu, BOOT_HANDSHAKE_ALLOWANCE, move |vcpu, deadline| {
        run_owned(vcpu, vm, gic, prepared, deadline)
    });
    drop(memory);
    match result? {
        TaskOutcome::Finished(outcome) => Ok(outcome),
        TaskOutcome::TimedOut => Err(Arm64BootError::message(
            "outer ARM64 command proof watchdog expired without a validated terminal frame",
        )),
    }
}

fn run_owned(
    mut vcpu: VcpuFd,
    vm: VmFd,
    gic: DeviceFd,
    prepared: PreparedCommand,
    deadline: &mut watchdog::DeadlineArm<Arm64CommandOutcome>,
) -> Result<Arm64CommandOutcome, Arm64BootError> {
    let result = run_loop(&mut vcpu, &vm, prepared, deadline);
    drop(vcpu);
    drop(gic);
    drop(vm);
    result
}

fn run_loop(
    vcpu: &mut VcpuFd,
    vm: &VmFd,
    mut prepared: PreparedCommand,
    deadline: &mut watchdog::DeadlineArm<Arm64CommandOutcome>,
) -> Result<Arm64CommandOutcome, Arm64BootError> {
    let command_deadline = prepared
        .timeout
        .checked_add(TERMINAL_GRACE)
        .ok_or_else(|| Arm64BootError::message("command terminal deadline overflow"))?;
    let mut collector = ResponseCollector::new(&prepared);
    let request = std::mem::take(&mut prepared.request);
    let mut control = ControlUart::new(VmIrqTrigger::new(vm), request);
    let mut console = Uart::new(ARM64_BOOT_SENTINEL.as_bytes());
    let mut received_hello = false;
    loop {
        match vcpu
            .run()
            .map_err(|error| Arm64BootError::at("run command vCPU 0", error))?
        {
            VcpuExit::MmioRead(address, data) if in_device(address, UART_BASE) => console
                .read(address, data)
                .map_err(|error| Arm64BootError::at("emulate diagnostic UART read", error))?,
            VcpuExit::MmioWrite(address, data) if in_device(address, UART_BASE) => {
                let _sentinel_seen = console
                    .write(address, data)
                    .map_err(|error| Arm64BootError::at("emulate diagnostic UART write", error))?;
                if let Some(stage) = pre_hello_diagnostic_failure(console.console(), received_hello)
                {
                    return Err(Arm64BootError::message(format!(
                        "trusted guest agent failed during {stage}"
                    )));
                }
            }
            VcpuExit::MmioRead(address, data) if in_device(address, CONTROL_UART_BASE) => {
                control.read(address, data)?;
            }
            VcpuExit::MmioWrite(address, data) if in_device(address, CONTROL_UART_BASE) => {
                if let Some(frame) = control.write(address, data)? {
                    if !received_hello {
                        validate_hello(&frame)?;
                        control.start_request()?;
                        deadline.arm(command_deadline)?;
                        received_hello = true;
                        console.stop_capture();
                    } else if !control.request_drained() {
                        return Err(Arm64BootError::message(
                            "guest responded before draining the complete command request",
                        ));
                    } else if let Some(outcome) = collector.accept(frame)? {
                        return Ok(outcome);
                    }
                }
            }
            VcpuExit::Intr => {}
            exit => {
                return Err(Arm64BootError::message(format!(
                    "vCPU exited before command terminal frame: {exit:?}"
                )));
            }
        }
    }
}

fn in_device(address: u64, base: u64) -> bool {
    base.checked_add(UART_SIZE)
        .is_some_and(|end| address >= base && address < end)
}

fn pre_hello_diagnostic_failure(console: &[u8], received_hello: bool) -> Option<&'static str> {
    const FAILURES: &[(&[u8], &str)] = &[
        (b"SOMA_AGENT_FAIL:standard-fds", "standard descriptor setup"),
        (b"SOMA_AGENT_FAIL:mount-devtmpfs", "devtmpfs mount"),
        (b"SOMA_AGENT_FAIL:open-control", "control UART open"),
        (b"SOMA_AGENT_FAIL:read-termios", "control UART termios read"),
        (
            b"SOMA_AGENT_FAIL:write-termios",
            "control UART raw-mode setup",
        ),
        (
            b"SOMA_AGENT_FAIL:unlink-control",
            "control UART node removal",
        ),
        (b"SOMA_AGENT_FAIL:send-hello", "agent Hello transmission"),
    ];
    if received_hello {
        return None;
    }
    FAILURES
        .iter()
        .find_map(|(tag, stage)| console.ends_with(tag).then_some(*stage))
}

#[cfg(test)]
mod tests {
    use super::pre_hello_diagnostic_failure;

    #[test]
    fn pre_hello_diagnostic_failure_exposes_only_fixed_suffixes() {
        assert_eq!(
            pre_hello_diagnostic_failure(b"noise SOMA_AGENT_FAIL:open-control", false),
            Some("control UART open")
        );
        assert_eq!(
            pre_hello_diagnostic_failure(b"SOMA_AGENT_FAIL:open-control forged", false),
            None
        );
        assert_eq!(
            pre_hello_diagnostic_failure(b"SOMA_AGENT_FAIL:open-control", true),
            None
        );
        assert_eq!(
            pre_hello_diagnostic_failure(b"guest-controlled secret", false),
            None
        );
    }
}
