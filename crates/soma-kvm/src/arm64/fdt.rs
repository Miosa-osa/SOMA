use vm_fdt::{Error, FdtWriter};

use super::layout::{
    BootLayout, CONTROL_UART_BASE, CONTROL_UART_SPI, GIC_DIST_BASE, GIC_DIST_SIZE, GIC_REDIST_BASE,
    GIC_REDIST_SIZE, RAM_BASE, RAM_SIZE, UART_BASE, UART_SIZE, UART_SPI,
};

const GIC_PHANDLE: u32 = 1;
const IRQ_SPI: u32 = 0;
const IRQ_PPI: u32 = 1;
const IRQ_LEVEL_HIGH: u32 = 4;
const IRQ_EDGE_RISING: u32 = 1;

pub(crate) fn build(layout: BootLayout, include_control: bool) -> Result<Vec<u8>, Error> {
    let mut writer = FdtWriter::new()?;
    let root = writer.begin_node("")?;
    writer.property_string("compatible", "linux,dummy-virt")?;
    writer.property_u32("#address-cells", 2)?;
    writer.property_u32("#size-cells", 2)?;
    writer.property_u32("interrupt-parent", GIC_PHANDLE)?;

    aliases(&mut writer, include_control)?;
    cpu(&mut writer)?;
    memory(&mut writer)?;
    chosen(&mut writer, layout)?;
    gic(&mut writer)?;
    timer(&mut writer)?;
    psci(&mut writer)?;
    uart(
        &mut writer,
        "uart@9000000",
        UART_BASE,
        UART_SPI,
        IRQ_LEVEL_HIGH,
    )?;
    if include_control {
        uart(
            &mut writer,
            "uart@9010000",
            CONTROL_UART_BASE,
            CONTROL_UART_SPI,
            IRQ_EDGE_RISING,
        )?;
    }

    writer.end_node(root)?;
    writer.finish()
}

fn aliases(writer: &mut FdtWriter, include_control: bool) -> Result<(), Error> {
    let aliases = writer.begin_node("aliases")?;
    writer.property_string("serial0", "/uart@9000000")?;
    if include_control {
        writer.property_string("serial1", "/uart@9010000")?;
    }
    writer.end_node(aliases)
}

fn cpu(writer: &mut FdtWriter) -> Result<(), Error> {
    let cpus = writer.begin_node("cpus")?;
    writer.property_u32("#address-cells", 2)?;
    writer.property_u32("#size-cells", 0)?;
    let cpu = writer.begin_node("cpu@0")?;
    writer.property_string("device_type", "cpu")?;
    writer.property_string("compatible", "arm,arm-v8")?;
    writer.property_string("enable-method", "psci")?;
    writer.property_u64("reg", 0)?;
    writer.end_node(cpu)?;
    writer.end_node(cpus)
}

fn memory(writer: &mut FdtWriter) -> Result<(), Error> {
    let memory = writer.begin_node("memory@80000000")?;
    writer.property_string("device_type", "memory")?;
    writer.property_array_u64("reg", &[RAM_BASE, RAM_SIZE])?;
    writer.end_node(memory)
}

fn chosen(writer: &mut FdtWriter, layout: BootLayout) -> Result<(), Error> {
    let chosen = writer.begin_node("chosen")?;
    writer.property_string(
        "bootargs",
        "console=ttyS0,115200n8 earlycon=uart8250,mmio,0x09000000,115200n8 rdinit=/init panic=-1 nokaslr",
    )?;
    writer.property_string("stdout-path", "/uart@9000000")?;
    writer.property_u64("linux,initrd-start", layout.initrd_start)?;
    writer.property_u64("linux,initrd-end", layout.initrd_end)?;
    writer.end_node(chosen)
}

fn gic(writer: &mut FdtWriter) -> Result<(), Error> {
    let gic = writer.begin_node("intc@8000000")?;
    writer.property_string("compatible", "arm,gic-v3")?;
    writer.property_null("interrupt-controller")?;
    writer.property_u32("#interrupt-cells", 3)?;
    writer.property_phandle(GIC_PHANDLE)?;
    writer.property_array_u64(
        "reg",
        &[
            GIC_DIST_BASE,
            GIC_DIST_SIZE,
            GIC_REDIST_BASE,
            GIC_REDIST_SIZE,
        ],
    )?;
    writer.end_node(gic)
}

fn timer(writer: &mut FdtWriter) -> Result<(), Error> {
    let timer = writer.begin_node("timer")?;
    writer.property_string("compatible", "arm,armv8-timer")?;
    writer.property_null("always-on")?;
    writer.property_array_u32(
        "interrupts",
        &[
            IRQ_PPI,
            13,
            IRQ_LEVEL_HIGH,
            IRQ_PPI,
            14,
            IRQ_LEVEL_HIGH,
            IRQ_PPI,
            11,
            IRQ_LEVEL_HIGH,
            IRQ_PPI,
            10,
            IRQ_LEVEL_HIGH,
        ],
    )?;
    writer.end_node(timer)
}

fn psci(writer: &mut FdtWriter) -> Result<(), Error> {
    let psci = writer.begin_node("psci")?;
    writer.property_string("compatible", "arm,psci-0.2")?;
    writer.property_string("method", "hvc")?;
    writer.end_node(psci)
}

fn uart(
    writer: &mut FdtWriter,
    name: &str,
    base: u64,
    spi: u32,
    trigger: u32,
) -> Result<(), Error> {
    let uart = writer.begin_node(name)?;
    writer.property_string("compatible", "ns16550a")?;
    writer.property_array_u64("reg", &[base, UART_SIZE])?;
    writer.property_u32("clock-frequency", 24_000_000)?;
    writer.property_u32("current-speed", 115_200)?;
    writer.property_array_u32("interrupts", &[IRQ_SPI, spi, trigger])?;
    writer.end_node(uart)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_boot_dtb_does_not_advertise_the_unemulated_control_uart() {
        let layout = BootLayout::new(4096).unwrap();
        let cold = build(layout, false).unwrap();
        let command = build(layout, true).unwrap();
        let name = b"uart@9010000";
        assert!(!cold.windows(name.len()).any(|window| window == name));
        assert!(command.windows(name.len()).any(|window| window == name));
    }
}
