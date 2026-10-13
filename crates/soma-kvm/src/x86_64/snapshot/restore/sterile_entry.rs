//! The sterile restore entry point, split out to hold its file inside the source budget.
#[expect(
    clippy::wildcard_imports,
    reason = "this module is its parent's body moved wholesale; naming every import would obscure it"
)]
use super::*;

/// Restores one machine that holds no Instance authority yet.
///
/// # Errors
///
/// Returns the first typed failure; every resource acquired before it is released in reverse
/// ownership order as the partially built machine unwinds.
#[allow(clippy::too_many_lines)]
pub fn restore_sterile(request: SterileRequest) -> Result<Sterile, SnapshotError> {
    let SterileRequest {
        mut objects,
        hypervisor,
        root,
        overlay_capacity_bytes,
        volume_capacity_bytes,
        devices,
        memory_bytes,
        vcpus,
        contract,
        verify_artifacts,
        hotplug,
    } = request;
    let mut timeline = Timeline::new();
    let mut sequence = RestoreSequence::start();
    let kvm = hypervisor
        .handle()
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    let state_bytes = objects.state_bytes()?;
    let snapshot = Digest::of(&state_bytes);
    let manifest = Manifest::decode(&state_bytes)?;
    // The device set comes from the Generation being launched, never from the snapshot: a set
    // read out of the artifact would agree with itself, and the point of the check is that the
    // machine this host means to build and the machine the snapshot describes are the same one.
    let profile = profile::host_profile(&kvm, memory_bytes, vcpus, contract, devices)?;
    compatibility::check(&profile, &manifest)?;
    compatibility::check_hotplug(hotplug, &manifest)?;
    let state = Sections::read(&manifest, devices, vcpus)?;
    let repair_point_line = marker::decode(section(&manifest, SectionRole::RepairPointMarker)?)?;
    if verify_artifacts {
        verify(&mut objects, &manifest, &state)?;
    }
    sequence.complete(RestoreStep::ValidateManifest)?;
    timeline.mark(Milestone::ValidateManifest);

    let vm = kvm
        .create_vm()
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    sequence.complete(RestoreStep::CreateVm)?;
    timeline.mark(Milestone::CreateVm);

    let mut ram = guest_memory::map(objects.memory(), &manifest, &mut timeline)?;
    // The region is never in the memory object, so it restores empty from its geometry alone.
    if let Some(region) = hotplug {
        ram.attach_hotplug(region)?;
    }
    sequence.complete(RestoreStep::MapMemoryPrivately)?;
    timeline.mark(Milestone::MapMemory);

    // The launch page is a separate slot the snapshot never contained, and it stays empty
    // until `write_launch_page` publishes the material just before the resume. It is added
    // while the VM still has no vCPU, because that is what a memory-slot addition costs; the
    // identical call after the vCPU exists costs two milliseconds. It is also bound before the
    // machine adopts the VM, so on any later failure the VM is released before this mapping
    // is, which is the ownership order `RamMapping` relies on.
    let launch_page = LaunchPageSlot::map_and_register(&vm)?;
    timeline.mark(Milestone::LaunchPageMapped);

    let machine = Machine::adopt(kvm, vm, ram);
    machine.register_certified_slots(&state.vm)?;
    sequence.complete(RestoreStep::RegisterMemorySlots)?;
    timeline.mark(Milestone::RegisterSlots);

    let mac = net_mac(state.slot(Slot::Net))?;
    let captured_cid = vsock_cid(state.slot(Slot::Vsock))?;
    machine.recreate_platform(&state.vm, &state.routing)?;
    timeline.mark(Milestone::Platform);
    let bus = recreate_devices(
        &machine,
        root,
        &state,
        &Identity { mac, captured_cid },
        devices::Declared {
            overlay_capacity_bytes,
            volume_capacity_bytes,
            set: devices,
            contract,
        },
    )?;
    sequence.complete(RestoreStep::RecreateIrqchipAndDevices)?;
    timeline.mark(Milestone::Devices);

    // One descriptor per certified processor, and each carries the section captured for that
    // index: a machine restored with the wrong processor's registers would come back believing
    // it is a different one.
    let mut vcus = Vec::with_capacity(state.vcpus.len());
    for (index, captured) in state.vcpus.iter().enumerate() {
        let vcpu = machine
            .vm_fd()
            .create_vcpu(
                u64::try_from(index)
                    .map_err(|_| MachineError::invalid(Phase::Restore, "vCPU index overflow"))?,
            )
            .map_err(|error| MachineError::os(Phase::Restore, error))?;
        vcpu::write_configuration(machine.kvm_fd(), &vcpu, captured)?;
        vcpu::write_registers(machine.kvm_fd(), &vcpu, captured)?;
        vcus.push(vcpu);
    }
    sequence.complete(RestoreStep::CreateVcpu)?;
    timeline.mark(Milestone::Vcpu);
    sequence.complete(RestoreStep::RestoreCpuidAndMsrs)?;
    sequence.complete(RestoreStep::RestoreVcpuState)?;
    timeline.mark(Milestone::VcpuRestored);

    let serial_line = EventFd::new(libc::EFD_NONBLOCK)
        .map_err(|error| MachineError::io(Phase::Restore, &error))?;
    machine
        .vm_fd()
        .register_irqfd(&serial_line, SERIAL_GSI)
        .map_err(|error| MachineError::os(Phase::Restore, error))?;
    let mut irq = IrqLines::create(devices)?;
    irq.register(machine.vm_fd())?;
    let notify = NotifyFds::register(machine.vm_fd(), &bus)?;
    // Every route exists before any captured interrupt state is armed.
    platform::write_irqchip(machine.vm_fd(), &state.irqchip)?;
    platform::write_pit(machine.vm_fd(), state.pit)?;
    // Frozen: the paused clock, never realtime-advanced - see ClockState::frozen.
    platform::write_clock(machine.vm_fd(), state.clock.frozen())?;
    sequence.complete(RestoreStep::RestoreDeviceAndInterruptState)?;
    timeline.mark(Milestone::Events);

    let machine = SandboxMachine::from_restored(RestoredParts {
        machine,
        bus,
        vcpus: vcus,
        serial_line,
        irq,
        notify,
        launch_page,
        clock: Stopwatch::new(),
        timeline,
        cmdline: crate::cmdline::compose_generation_for(devices, contract),
    })?;
    Ok(Sterile {
        machine,
        facts: SterileFacts {
            snapshot,
            candidate_id: *manifest.header().candidate_id.as_bytes(),
            memory_bytes: manifest.header().memory.size(),
            repair_point_line,
            mac,
            captured_cid,
            vcpus: manifest.header().vcpu_count,
        },
        sequence,
    })
}
