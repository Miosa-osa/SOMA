//! Read the command line into the build this example runs.

use super::{Args, BuildInputs, DEFAULT_MEMORY_MIB, DEFAULT_STORAGE_MIB, PathBuf, build};

pub(super) fn parse_args() -> Result<Args, String> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.len() < 8 || raw.len() > 10 {
        return Err(format!(
            "expected 8 to 10 arguments, got {}\n\
             usage: prepare_generation <reference> <oci-layout> <kernel> <kernel-config> \
             <guest-agent> <erofs-tools> <e2fsprogs> <out-entry> [memory_mib] [storage_mib]",
            raw.len()
        ));
    }
    let number = |value: &str, name: &str| {
        value
            .parse::<u64>()
            .map_err(|_| format!("{name} must be a positive integer, got {value:?}"))
    };
    let vcpus = std::env::var("SOMA_PREPARE_VCPUS").map_or(Ok(1_u16), |value| {
        value
            .parse::<u16>()
            .map_err(|_| format!("SOMA_PREPARE_VCPUS must be a positive integer, got {value:?}"))
    })?;
    if vcpus == 0 {
        return Err("SOMA_PREPARE_VCPUS must be at least one".to_owned());
    }
    let network = match std::env::var("SOMA_PREPARE_NETWORK").as_deref() {
        Err(_) | Ok("" | "isolated") => false,
        Ok("public_internet") => true,
        Ok(other) => {
            return Err(format!(
                "SOMA_PREPARE_NETWORK must be isolated or public_internet, got {other:?}"
            ));
        }
    };
    let ceiling_mib = build::ceiling_from_environment()?;
    Ok(Args {
        reference: raw[0].clone(),
        inputs: BuildInputs {
            layout: PathBuf::from(&raw[1]),
            kernel: PathBuf::from(&raw[2]),
            kernel_config: PathBuf::from(&raw[3]),
            agent: PathBuf::from(&raw[4]),
            erofs_tools: PathBuf::from(&raw[5]),
            e2fsprogs: PathBuf::from(&raw[6]),
            out_entry: PathBuf::from(&raw[7]),
            ceiling_mib,
        },
        memory_mib: raw
            .get(8)
            .map_or(Ok(DEFAULT_MEMORY_MIB), |v| number(v, "memory_mib"))?,
        storage_mib: raw
            .get(9)
            .map_or(Ok(DEFAULT_STORAGE_MIB), |v| number(v, "storage_mib"))?,
        vcpus,
        network,
    })
}
