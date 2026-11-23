use super::*;
use soma_kvm::DeviceSet;

#[test]
fn version_two_statement_records_the_mp_table_and_the_no_noapic_line() {
    let text = core::str::from_utf8(MACHINE_CONTRACT_V2).unwrap();
    assert!(text.starts_with("soma-x86_64-machine-contract-v2\n"));
    assert!(text.contains("vcpu=1-8"));
    assert!(text.contains("ram-max=17179869184"));
    // The declared address is the one the encoder writes to, so the statement cannot drift
    // from the machine without failing here.
    assert!(text.contains(&format!(
        "mptable={:#x}",
        soma_kvm::mptable::MP_TABLE_ADDRESS
    )));
    assert!(!text.contains("noapic"));
    assert!(text.contains("base-cmdline=console=ttyS0"));
}

#[test]
fn version_one_statement_keeps_the_fields_that_make_its_digest() {
    let text = core::str::from_utf8(MACHINE_CONTRACT_V1).unwrap();
    assert!(text.starts_with("soma-x86_64-machine-contract-v1\n"));
    assert!(text.contains("vcpu=1 ram-min=134217728 ram-max=3221225472 ram-step=4096"));
    assert!(text.ends_with("noapic cryptomgr.notests\n"));
}

#[test]
fn command_line_selects_the_contract_base_argument_set() {
    let v1 = kernel_command_line_v1(DeviceSet::FULL);
    let v2 = kernel_command_line_v2(DeviceSet::FULL);
    assert!(String::from_utf8_lossy(&v1).contains(" noapic "));
    assert!(!String::from_utf8_lossy(&v2).contains("noapic"));
    assert_eq!(
        kernel_command_line_for(DeviceSet::FULL, 1).as_deref(),
        Some(v1.as_slice())
    );
    assert_eq!(
        kernel_command_line_for(DeviceSet::FULL, 2).as_deref(),
        Some(v2.as_slice())
    );
    let v3 = kernel_command_line_for(DeviceSet::FULL, 3).expect("version 3 has a line");
    // The memory device comes from the device set, not from the contract.
    assert!(!String::from_utf8_lossy(&v3).contains("0xd0005000"));
    let with_mem = kernel_command_line_for(DeviceSet::FULL_WITH_MEM, 3).expect("a line");
    assert!(
        String::from_utf8_lossy(&with_mem).contains("virtio_mmio.device=4K@0xd0005000:10:5")
    );
    assert_ne!(
        device_contract_statement(DeviceSet::FULL),
        device_contract_statement(DeviceSet::FULL_WITH_MEM)
    );
    assert!(
        String::from_utf8_lossy(&device_contract_statement(DeviceSet::FULL_WITH_MEM))
            .contains("slot5 mmio=0xd0005000-0xd0005fff gsi=10 device=mem id=24")
    );
    assert_eq!(kernel_command_line_for(DeviceSet::FULL, 4), Some(v3));
    assert_eq!(
        kernel_command_line_for(DeviceSet::FULL, 5),
        kernel_command_line_for(DeviceSet::FULL, 4)
    );
    assert_eq!(
        kernel_command_line_for(DeviceSet::FULL, 6),
        kernel_command_line_for(DeviceSet::FULL, 5)
    );
    assert_eq!(kernel_command_line_for(DeviceSet::FULL, 7), None);
}

#[test]
fn the_slot_layout_versions_restate_the_machine_geometry() {
    // A restatement is only safe while a test binds it to the thing it restates.
    assert_eq!(
        MEMORY_SLOT_LAYOUT_VERSION,
        soma_kvm::memory_layout::LAYOUT_VERSION
    );
    assert_eq!(
        MEMORY_SLOT_LAYOUT_VERSION_V2,
        soma_kvm::memory_layout::SPLIT_LAYOUT_VERSION
    );
    assert_eq!(MEMORY_SLOT_LAYOUT_VERSION, 1);
    assert_eq!(
        memory_slot_layout_version(3 * 1024 * 1024 * 1024),
        MEMORY_SLOT_LAYOUT_VERSION
    );
    assert_eq!(
        memory_slot_layout_version(16 * 1024 * 1024 * 1024),
        MEMORY_SLOT_LAYOUT_VERSION_V2
    );
}

#[test]
fn contract_binding_selection_refuses_unknown_versions() {
    assert_eq!(machine_contract_for(1), Some(machine_contract_v1()));
    assert_eq!(machine_contract_for(2), Some(machine_contract_v2()));
    assert_eq!(machine_contract_for(0), None);
    assert_eq!(machine_contract_for(3), Some(machine_contract_v3()));
    assert_eq!(
        machine_contract_for(4),
        Some(ContractBinding::of(4, MACHINE_CONTRACT_V4))
    );
    assert_eq!(
        machine_contract_for(5),
        Some(ContractBinding::of(5, MACHINE_CONTRACT_V5))
    );
    assert_eq!(
        machine_contract_for(6),
        Some(ContractBinding::of(6, MACHINE_CONTRACT_V6))
    );
    assert_eq!(machine_contract_for(7), None);
    assert_ne!(
        machine_contract_v3(),
        ContractBinding::of(4, MACHINE_CONTRACT_V4)
    );
    assert_ne!(machine_contract_v1(), machine_contract_v2());
    assert_ne!(machine_contract_v2(), machine_contract_v3());
    assert_eq!(machine_contract_v3().version, 3);
    assert_eq!(machine_contract_v1().version, 1);
    assert_eq!(machine_contract_v2().version, 2);
}
