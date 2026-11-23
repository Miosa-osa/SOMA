use super::{Paused, Stage};

#[test]
fn repeated_pause_keeps_the_existing_pause_evidence() {
    let mut stage = paused_stage(true);
    let Stage::Paused(paused) = &stage else {
        unreachable!()
    };
    let original = std::ptr::from_ref(paused.as_ref());

    assert!(matches!(stage.begin_pause(), Ok(None)));
    let Stage::Paused(paused) = &stage else {
        panic!("the paused state was discarded")
    };
    assert_eq!(std::ptr::from_ref(paused.as_ref()), original);
}

#[test]
fn retry_cannot_turn_an_incomplete_pause_into_success() {
    let mut stage = paused_stage(false);

    assert!(stage.begin_pause().is_err());
    assert!(matches!(stage, Stage::Paused(paused) if !paused.complete));
}

#[test]
#[ignore = "requires a Linux x86_64 host with accessible /dev/kvm"]
fn live_pause_freezes_guest_registers_and_memory() {
    use std::time::{Duration, Instant};

    use super::super::testing::spinning_machine;
    use super::{GuestExit, SandboxMachine};
    use crate::MachineContract;
    use crate::virtio::{GuestAddress, GuestMemory as _};
    use crate::x86_64::{EXPECTED_SERIAL, layout::KERNEL_START};

    const GRACE: Duration = Duration::from_secs(2);
    let contract = MachineContract::V1;
    let entry = u32::try_from(KERNEL_START).unwrap();
    let (mut machine, counter_address) = spinning_machine(None);
    assert!(machine.pause(GRACE).is_err());
    machine.watch_console(EXPECTED_SERIAL);
    machine.start().expect("boot the same guest");
    machine
        .wait_console_line(Instant::now() + GRACE)
        .expect("guest reached the spin loop");
    std::thread::sleep(Duration::from_millis(10));

    machine.pause(GRACE).expect("pause the running guest");
    let read_state = |machine: &mut SandboxMachine| {
        let paused = machine.paused().expect("resident machine");
        let registers = paused.vcpus[0].get_regs().unwrap();
        let mut counter = [0; 4];
        paused
            .memory
            .read_bytes(GuestAddress(u64::from(counter_address)), &mut counter)
            .unwrap();
        (registers.rip, u32::from_le_bytes(counter))
    };
    let first = read_state(&mut machine);
    assert_checkpoint_cpu_state(&mut machine, first.0, contract.min_memory_bytes());
    assert_ne!(first.1, 0, "the guest never advanced its counter");
    std::thread::sleep(Duration::from_millis(50));
    machine.pause(GRACE).expect("retry the confirmed pause");
    assert!(
        machine.start().is_err(),
        "start must preserve the paused machine"
    );
    assert_eq!(read_state(&mut machine), first);
    println!("pause confirmed: rip={:#x} counter={}", first.0, first.1);
    machine.resume().expect("resume the same machine");
    machine.resume().expect("retry the completed resume");
    std::thread::sleep(Duration::from_millis(10));
    machine.pause(GRACE).expect("pause the same guest again");
    let second = read_state(&mut machine);
    assert!(
        second.1 > first.1,
        "resident counter did not advance: {first:?} -> {second:?}"
    );
    assert!(
        (u64::from(entry)..u64::from(counter_address)).contains(&second.0),
        "guest left the original program: {second:?}"
    );
    println!("resumed counter: {} -> {}", first.1, second.1);
    let evidence = machine.finish(Duration::ZERO);
    assert_eq!(evidence.exit, Ok(GuestExit::Paused));
    assert_eq!(
        evidence.serial, EXPECTED_SERIAL,
        "the same console emitted its boot marker once"
    );
}

fn assert_checkpoint_cpu_state(
    machine: &mut super::SandboxMachine,
    expected_rip: u64,
    expected_memory: u64,
) {
    use crate::snapshot::capture::CaptureStep;
    use crate::x86_64::snapshot::vm_state::PausedVmState;

    let paused = machine.paused().expect("resident machine");
    let mut order = Vec::new();
    let captured = PausedVmState::read(
        paused.kvm,
        paused.vm,
        &paused.vcpus,
        paused.ram_bytes,
        |step| {
            order.push(step);
            Ok(())
        },
    )
    .expect("capture KVM state after the vCPU paused");
    assert_eq!(order, CaptureStep::ORDER[..6]);
    assert_eq!(captured.vcpus[0].regs().rip, expected_rip);
    assert_eq!(captured.vm.total_bytes(), expected_memory);
    println!("canonical checkpoint state read while the original VM was stopped");
}

fn paused_stage(complete: bool) -> Stage {
    Stage::Paused(Box::new(Paused {
        reports: Vec::new(),
        devices: None,
        complete,
        context: None,
    }))
}
